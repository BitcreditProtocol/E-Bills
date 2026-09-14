use std::sync::Arc;

use bcr_ebill_api::service::transport_service::transport_client::TransportClientApi;
use bcr_ebill_core::protocol::{
    BlockId, Sha256Hash, Timestamp,
    blockchain::{
        Block, BlockchainType, bill::BillBlock, company::CompanyBlock, identity::IdentityBlock,
    },
    crypto::BcrKeys,
    event::{BillBlockEvent, CompanyBlockEvent, IdentityBlockEvent},
};
use bcr_ebill_persistence::nostr::NostrChainEvent;
use nostr::{
    event::EventId,
    nips::nip10::{Marker, Nip10Tag},
};

use crate::transport::{decrypt_or_decode_public_chain_event, unwrap_public_chain_event};
use bcr_ebill_api::service::transport_service::{Error, Result};

/// Compares two chains for sorting. Ordering is:
/// 1. Length descending (longer chain wins)
/// 2. Tip timestamp ascending (earlier wins)
/// 3. Tip hash ascending (deterministic tiebreaker)
pub fn compare_chains(a: &[EventContainer], b: &[EventContainer]) -> std::cmp::Ordering {
    b.len()
        .cmp(&a.len())
        .then_with(|| {
            let a_ts = a
                .last()
                .map(|e| e.block.get_block_timestamp())
                .unwrap_or(Timestamp::now());
            let b_ts = b
                .last()
                .map(|e| e.block.get_block_timestamp())
                .unwrap_or(Timestamp::now());
            a_ts.cmp(&b_ts)
        })
        .then_with(|| {
            let a_hash = a.last().map(|e| e.block.get_block_hash());
            let b_hash = b.last().map(|e| e.block.get_block_hash());
            a_hash.cmp(&b_hash)
        })
}

/// Compares two unpacked chains for sorting, fork detection and fork point extraction. Returns
/// whether remote chain should be preferred over local chain and if remote is a fork an optional
/// fork point. When this function is changed we also need to change the `compare_chains` function
/// above as it determines the order in which remote chains are tested.
/// Precedence is:
/// 1. Longer chain wins
/// 2. Earlier tip  wins
/// 3. Fork point at divergent hash
pub fn resolve_fork<B: Block>(local: &[B], remote: &[B]) -> (bool, Option<BlockId>) {
    let remote_preferred = {
        let local_len = local.len();
        let remote_len = remote.len();
        if remote_len != local_len {
            remote_len > local_len
        } else {
            match (remote.last(), local.last()) {
                (Some(remote_tip), Some(local_tip)) => {
                    match remote_tip.timestamp().cmp(&local_tip.timestamp()) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Greater => false,
                        std::cmp::Ordering::Equal => remote_tip.hash() < local_tip.hash(),
                    }
                }
                (Some(_), None) => true,
                _ => false,
            }
        }
    };

    if !remote_preferred {
        return (false, None);
    }

    for (i, local_block) in local.iter().enumerate() {
        if let Some(remote_block) = remote.get(i) {
            if local_block.hash() != remote_block.hash() {
                let prev_hash_valid = if i == 0 {
                    false
                } else {
                    remote_block.previous_hash() == local[i - 1].hash()
                };
                if prev_hash_valid {
                    return (true, Some(local_block.id()));
                }
                return (false, None);
            }
        } else {
            return (false, None);
        }
    }

    (true, None)
}

/// Determines if the remote block can be considered a fork point in the chain:
/// - same id
/// - different hash
/// - lower timestamp or lower hash
pub fn is_fork_block<T: Block>(local: &T, remote: &T) -> bool {
    remote.id() == local.id()
        && remote.hash() != local.hash()
        && (remote.timestamp() < local.timestamp()
            || (remote.timestamp() == local.timestamp() && remote.hash() < local.hash()))
}

/// Will query the transport for the public chain events and build up as many chains as needed for
/// the Nostr chain structure. This does not look into the actual blockchain, but will build the
/// chains just from Nostr metadata.
pub async fn resolve_event_chains(
    transport: Arc<dyn TransportClientApi>,
    chain_id: &str,
    chain_type: BlockchainType,
    keys: &Option<BcrKeys>,
) -> Result<Vec<Vec<EventContainer>>> {
    let events = transport.resolve_public_chain(chain_id, chain_type).await?;
    let mut chains = collect_event_chains(&events, chain_id, chain_type, keys);
    chains.sort_by(|a, b| compare_chains(a, b));
    Ok(chains)
}

// Will build up as many chains as needed for the Nostr chain structure. This does not look into
// the actual blockchain, but will build the chains just from Nostr metadata.
pub fn collect_event_chains(
    events: &[nostr::event::Event],
    chain_id: &str,
    chain_type: BlockchainType,
    keys: &Option<BcrKeys>,
) -> Vec<Vec<EventContainer>> {
    let mut result = Vec::new();
    let markers: Vec<EventContainer> = events
        .iter()
        .filter_map(|e| ids_and_markers(e, chain_id, chain_type, keys))
        .collect();
    if let Some((root, children)) = split_root(&markers) {
        result = root.resolve_children(&children).as_chains();
    }
    result
}

pub fn split_root(markers: &[EventContainer]) -> Option<(EventContainer, Vec<EventContainer>)> {
    if let Some(root) = markers.iter().find(|v| v.is_root()) {
        let remainder = markers.iter().filter(|v| !v.is_root()).cloned().collect();
        return Some((root.clone(), remainder));
    }
    None
}

// find root and reply note ids of given event
pub fn ids_and_markers(
    event: &nostr::event::Event,
    chain_id: &str,
    chain_type: BlockchainType,
    keys: &Option<BcrKeys>,
) -> Option<EventContainer> {
    if let Ok(block) = decode_block(event.clone(), chain_id, chain_type, keys) {
        let mut result = EventContainer::new(event.clone(), None, None, block);

        for tag in event
            .tags
            .iter()
            .filter_map(|tag| Nip10Tag::try_from(tag).ok())
        {
            let Nip10Tag::Event { id, marker, .. } = tag;
            match marker {
                Some(Marker::Root) => result.root_id = Some(id.to_owned()),
                Some(Marker::Reply) => result.reply_id = Some(id.to_owned()),
                _ => {}
            }
        }
        Some(result)
    } else {
        None
    }
}

pub fn decode_block(
    event: nostr::event::Event,
    chain_id: &str,
    chain_type: BlockchainType,
    keys: &Option<BcrKeys>,
) -> Result<BlockData> {
    if let Ok(Some(payload)) = unwrap_public_chain_event(&event) {
        if (payload.id == chain_id) && (payload.chain_type == chain_type) {
            let decoded = decrypt_or_decode_public_chain_event(&payload.payload, keys)?;
            let data = match chain_type {
                BlockchainType::Bill => {
                    BlockData::Bill(borsh::from_slice::<BillBlockEvent>(&decoded.data)?.block)
                }
                BlockchainType::Identity => BlockData::Identity(
                    borsh::from_slice::<IdentityBlockEvent>(&decoded.data)?.block,
                ),
                BlockchainType::Company => {
                    BlockData::Company(borsh::from_slice::<CompanyBlockEvent>(&decoded.data)?.block)
                }
            };
            Ok(data)
        } else {
            Err(Error::Blockchain(format!(
                "Invalid blockchain event {} {} expected {chain_id} {chain_type}",
                payload.id, payload.chain_type
            )))
        }
    } else {
        Err(Error::Blockchain(
            "Could not unwrap payload from public chain event".to_string(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockData {
    Bill(BillBlock),
    Identity(IdentityBlock),
    Company(CompanyBlock),
}

impl BlockData {
    pub fn get_block_height(&self) -> u64 {
        match self {
            BlockData::Bill(block) => block.id.inner(),
            BlockData::Identity(block) => block.id.inner(),
            BlockData::Company(block) => block.id.inner(),
        }
    }
    pub fn get_block_hash(&self) -> Sha256Hash {
        match self {
            BlockData::Bill(block) => block.hash.clone(),
            BlockData::Identity(block) => block.hash.clone(),
            BlockData::Company(block) => block.hash.clone(),
        }
    }
    pub fn get_block_timestamp(&self) -> Timestamp {
        match self {
            BlockData::Bill(block) => block.timestamp,
            BlockData::Identity(block) => block.timestamp,
            BlockData::Company(block) => block.timestamp,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EventContainer {
    pub event: nostr::event::Event,
    pub root_id: Option<EventId>,
    pub reply_id: Option<EventId>,
    pub children: Vec<EventContainer>,
    pub block: BlockData,
    pub block_height: u64,
}

impl EventContainer {
    pub fn new(
        event: nostr::event::Event,
        root_id: Option<EventId>,
        reply_id: Option<EventId>,
        block: BlockData,
    ) -> Self {
        let block_height = block.get_block_height();
        Self {
            event,
            root_id,
            reply_id,
            children: Vec::new(),
            block,
            block_height,
        }
    }

    pub fn is_root(&self) -> bool {
        self.root_id.is_none() && self.reply_id.is_none()
    }

    pub fn is_child_of(&self, event_id: &EventId) -> bool {
        match self.reply_id {
            Some(id) => &id == event_id,
            None => self.root_id.filter(|i| i == event_id).is_some(),
        }
    }

    pub fn with_children(self, children: Vec<EventContainer>) -> Self {
        Self {
            event: self.event,
            root_id: self.root_id,
            reply_id: self.reply_id,
            children,
            block: self.block,
            block_height: self.block_height,
        }
    }

    // given all markers recursively resolves all children
    pub fn resolve_children(self, markers: &[EventContainer]) -> Self {
        let mut children = Vec::new();
        markers
            .iter()
            .filter(|m| m.is_child_of(&self.event.id) && m.event.id != self.event.id)
            .for_each(|m| {
                let mut marker = m.clone();
                if m.children.is_empty() {
                    marker = m.clone().resolve_children(markers);
                }
                children.push(marker);
            });
        self.with_children(children)
    }

    // creates one or multiple chains from tree structure
    pub fn as_chains(&self) -> Vec<Vec<Self>> {
        if self.children.is_empty() {
            return vec![vec![self.clone()]];
        }

        let mut result = Vec::new();
        for child in self.children.iter() {
            let local_chain = vec![self.clone()];
            let chains = child.as_chains();
            for mut chain in chains {
                let mut current = local_chain.clone();
                current.append(&mut chain);
                result.push(current);
            }
        }
        result
    }

    pub fn as_chain_store_event(
        &self,
        chain_id: &str,
        chain_type: BlockchainType,
        block_height: usize,
    ) -> NostrChainEvent {
        NostrChainEvent {
            event_id: self.event.id.to_string(),
            root_id: self.root_id.unwrap_or(self.event.id).to_string(),
            reply_id: self.reply_id.map(|id| id.to_string()),
            author: self.event.pubkey.to_string(),
            chain_id: chain_id.to_string(),
            chain_type,
            block_height,
            block_hash: self.block.get_block_hash(),
            received: Timestamp::now(),
            time: self.event.created_at.into(),
            payload: self.event.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::core::BillId;
    use bcr_ebill_core::protocol::{
        BlockId, Sha256Hash, Timestamp,
        blockchain::bill::{BillOpCode, block::BillBlock},
        crypto::BcrKeys,
    };
    use nostr::event::FinalizeEvent;
    use std::str::FromStr;

    fn bill_id_test() -> BillId {
        BillId::new(
            bitcoin::secp256k1::PublicKey::from_str(
                "026423b7d36d05b8d50a89a1b4ef2a06c88bcd2c5e650f25e122fa682d3b39686c",
            )
            .unwrap(),
            bitcoin::Network::Testnet,
        )
    }

    fn make_test_block(
        block_id: BlockId,
        prev_hash: Sha256Hash,
        timestamp: Timestamp,
    ) -> BillBlock {
        BillBlock::new(
            bill_id_test(),
            block_id,
            prev_hash,
            Vec::new(),
            BillOpCode::Issue,
            &BcrKeys::new(),
            None,
            &BcrKeys::new(),
            timestamp,
            Sha256Hash::new("test plaintext hash"),
        )
        .unwrap()
    }

    fn make_event_container(block: BillBlock) -> EventContainer {
        let nostr_event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&nostr::key::Keys::generate())
            .expect("test event");
        EventContainer::new(nostr_event, None, None, BlockData::Bill(block))
    }

    fn sort_chains(chains: &mut [Vec<EventContainer>]) {
        chains.sort_by(|a, b| compare_chains(a, b));
    }

    #[test]
    fn test_get_block_timestamp() {
        let ts = Timestamp::new(1731593928).unwrap();
        let block = make_test_block(BlockId::first(), Sha256Hash::new("genesis"), ts);
        let block_data = BlockData::Bill(block);
        assert_eq!(
            block_data.get_block_timestamp(),
            Timestamp::new(1731593928).unwrap()
        );
    }

    #[test]
    fn test_sort_different_length_chains() {
        let ts = Timestamp::new(1731593928).unwrap();
        let block1 = make_test_block(BlockId::first(), Sha256Hash::new("genesis"), ts);
        let block2 = make_test_block(
            BlockId::next_from_previous_block_id(&block1.id),
            block1.hash.clone(),
            Timestamp::new(1731593929).unwrap(),
        );

        // chain_a has 2 blocks, chain_b has 1 block
        let chain_a = vec![make_event_container(block1), make_event_container(block2)];
        let chain_b = vec![make_event_container(make_test_block(
            BlockId::first(),
            Sha256Hash::new("genesis2"),
            ts,
        ))];

        let mut chains = vec![chain_b, chain_a];
        sort_chains(&mut chains);

        // Longer chain (length 2) should come first
        assert_eq!(chains[0].len(), 2);
        assert_eq!(chains[1].len(), 1);
    }

    #[test]
    fn test_sort_equal_length_earlier_timestamp_wins() {
        let ts_early = Timestamp::new(1731593900).unwrap();
        let ts_late = Timestamp::new(1731593999).unwrap();

        let chain_a = vec![make_event_container(make_test_block(
            BlockId::first(),
            Sha256Hash::new("genesis_a"),
            ts_early,
        ))];
        let chain_b = vec![make_event_container(make_test_block(
            BlockId::first(),
            Sha256Hash::new("genesis_b"),
            ts_late,
        ))];

        let mut chains = vec![chain_b, chain_a];
        sort_chains(&mut chains);

        // Earlier timestamp should come first
        assert_eq!(
            chains[0][0].block.get_block_timestamp(),
            Timestamp::new(1731593900).unwrap()
        );
        assert_eq!(
            chains[1][0].block.get_block_timestamp(),
            Timestamp::new(1731593999).unwrap()
        );
    }

    #[test]
    fn test_sort_equal_length_two_blocks_earlier_timestamp_wins() {
        let ts = Timestamp::new(1731593928).unwrap();
        let ts_early = Timestamp::new(1731593950).unwrap();
        let ts_late = Timestamp::new(1731593999).unwrap();

        // Shared first block
        let block1 = make_test_block(BlockId::first(), Sha256Hash::new("genesis"), ts);

        // Chain A: block1 -> block2a (earlier tip timestamp)
        let block2a = make_test_block(
            BlockId::next_from_previous_block_id(&block1.id),
            block1.hash.clone(),
            ts_early,
        );
        let chain_a = vec![
            make_event_container(block1.clone()),
            make_event_container(block2a),
        ];

        // Chain B: block1 -> block2b (later tip timestamp)
        let block2b = make_test_block(
            BlockId::next_from_previous_block_id(&block1.id),
            block1.hash.clone(),
            ts_late,
        );
        let chain_b = vec![make_event_container(block1), make_event_container(block2b)];

        // Put chain_b first to verify sorting reorders them
        let mut chains = vec![chain_b, chain_a];
        sort_chains(&mut chains);

        // Earlier tip timestamp should come first
        assert_eq!(chains[0].len(), 2);
        assert_eq!(chains[1].len(), 2);
        assert_eq!(
            chains[0].last().unwrap().block.get_block_timestamp(),
            ts_early
        );
        assert_eq!(
            chains[1].last().unwrap().block.get_block_timestamp(),
            ts_late
        );
    }

    #[test]
    fn test_sort_equal_length_same_timestamp_smaller_hash_wins() {
        let ts = Timestamp::new(1731593928).unwrap();

        // Different BcrKeys::new() generates different keys → different hashes
        let chain_a = vec![make_event_container(make_test_block(
            BlockId::first(),
            Sha256Hash::new("genesis_x"),
            ts,
        ))];
        let chain_b = vec![make_event_container(make_test_block(
            BlockId::first(),
            Sha256Hash::new("genesis_y"),
            ts,
        ))];

        let mut chains = vec![chain_a, chain_b];
        sort_chains(&mut chains);

        // The chain with the smaller hash should come first
        assert!(chains[0][0].block.get_block_hash() <= chains[1][0].block.get_block_hash());
    }

    #[test]
    fn test_sort_single_chain_unchanged() {
        let ts = Timestamp::new(1731593928).unwrap();
        let block = make_test_block(BlockId::first(), Sha256Hash::new("genesis"), ts);
        let block_hash = block.hash.clone();
        let chain = vec![make_event_container(block)];

        let mut chains = vec![chain];
        sort_chains(&mut chains);

        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].len(), 1);
        assert_eq!(chains[0][0].block.get_block_hash(), block_hash);
    }

    fn make_test_chain_with_timestamps(timestamps: &[u64]) -> Vec<BillBlock> {
        let mut blocks = Vec::new();
        let mut prev_hash = Sha256Hash::new("genesis");
        let mut block_id = BlockId::first();

        for ts in timestamps.iter() {
            let block = make_test_block(block_id, prev_hash.clone(), Timestamp::new(*ts).unwrap());
            prev_hash = block.hash.clone();
            block_id = BlockId::next_from_previous_block_id(&block_id);
            blocks.push(block);
        }
        blocks
    }

    #[test]
    fn test_resolve_fork_remote_longer_no_divergence() {
        let shared = make_test_chain_with_timestamps(&[1000]);
        let local = shared.clone();
        let mut remote = shared.clone();
        remote.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(1001).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_local_longer_returns_not_preferred() {
        let shared = make_test_chain_with_timestamps(&[1000]);
        let mut local = shared.clone();
        let remote = shared.clone();
        local.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(1001).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(!is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_equal_length_earlier_remote_timestamp() {
        let shared = make_test_chain_with_timestamps(&[1000]);
        let mut local = shared.clone();
        let mut remote = shared.clone();
        local.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(2000).unwrap(),
        ));
        remote.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(1500).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(is_preferred);
        assert_eq!(fork_point, Some(local[1].id()));
    }

    #[test]
    fn test_resolve_fork_equal_length_later_remote_timestamp() {
        let shared = make_test_chain_with_timestamps(&[1000]);
        let mut local = shared.clone();
        let mut remote = shared.clone();
        local.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(1500).unwrap(),
        ));
        remote.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(2000).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(!is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_identical_chains_not_preferred() {
        let local = make_test_chain_with_timestamps(&[1000, 1500, 2000]);
        let remote = local.clone();
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(!is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_genesis_divergence_not_preferred() {
        let local = make_test_chain_with_timestamps(&[1000]);
        let remote = make_test_chain_with_timestamps(&[1001]);
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(!is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_middle_divergence() {
        let shared = make_test_chain_with_timestamps(&[1000, 1500]);
        let mut local = shared.clone();
        let mut remote = shared.clone();
        local.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[1].id()),
            shared[1].hash.clone(),
            Timestamp::new(2000).unwrap(),
        ));
        remote.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[1].id()),
            shared[1].hash.clone(),
            Timestamp::new(1800).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(is_preferred);
        assert_eq!(fork_point, Some(local[2].id()));
    }

    #[test]
    fn test_resolve_fork_empty_local_remote_preferred() {
        let local: Vec<BillBlock> = vec![];
        let remote = make_test_chain_with_timestamps(&[1000, 1500]);
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_empty_remote_not_preferred() {
        let local = make_test_chain_with_timestamps(&[1000, 1500]);
        let remote: Vec<BillBlock> = vec![];
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(!is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_both_empty_not_preferred() {
        let local: Vec<BillBlock> = vec![];
        let remote: Vec<BillBlock> = vec![];
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(!is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_hash_tiebreaker() {
        let shared = make_test_chain_with_timestamps(&[1000]);
        let mut local = shared.clone();
        let mut remote = shared.clone();
        local.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(1500).unwrap(),
        ));
        remote.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(1500).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        if remote[1].hash < local[1].hash {
            assert!(is_preferred);
            assert_eq!(fork_point, Some(local[1].id()));
        } else {
            assert!(!is_preferred);
            assert_eq!(fork_point, None);
        }
    }

    #[test]
    fn test_resolve_fork_remote_shorter_at_divergence_not_preferred() {
        let shared = make_test_chain_with_timestamps(&[1000, 1500]);
        let mut local = shared.clone();
        let remote = shared.clone();
        local.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[1].id()),
            shared[1].hash.clone(),
            Timestamp::new(2000).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(!is_preferred);
        assert_eq!(fork_point, None);
    }

    #[test]
    fn test_resolve_fork_single_vs_two_blocks_extension() {
        let shared = make_test_chain_with_timestamps(&[1000]);
        let local = shared.clone();
        let mut remote = shared.clone();
        remote.push(make_test_block(
            BlockId::next_from_previous_block_id(&shared[0].id()),
            shared[0].hash.clone(),
            Timestamp::new(1000).unwrap(),
        ));
        let (is_preferred, fork_point) = resolve_fork(&local, &remote);
        assert!(is_preferred);
        assert_eq!(fork_point, None);
    }
}
