use std::io::Write;

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_core::protocol::crypto::BcrKeys;
use bitcoin::hashes::{
    Hash,
    sha256::{self, Hash as Sha256HexHash},
};
use nostr::{
    event::{EventBuilder, FinalizeEvent, Kind, Tag},
    types::time::Timestamp,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Generic result type
pub type Result<T> = std::result::Result<T, super::Error>;

/// Generic error type
#[derive(Debug, Error)]
pub enum Error {
    /// all errors originating from interacting with the web api
    #[error("External File Storage Web API error: {0}")]
    Api(#[from] reqwest::Error),
    /// all errors originating from invalid urls
    #[error("External File Storage Invalid Relay Url Error")]
    InvalidRelayUrl,
    /// all errors originating from invalid hashes
    #[error("External File Storage Invalid Hash")]
    InvalidHash,
    /// all errors originating from hashing
    #[error("External File Storage Hash Error")]
    Hash,
    #[error("External File Storage Authorization Error: {0}")]
    Auth(String),
}

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait FileStorageClientApi: ServiceTraitBounds {
    /// Upload the given bytes, checking and returning the nostr_hash
    async fn upload(&self, relay_url: &url::Url, bytes: Vec<u8>) -> Result<Sha256HexHash>;
    async fn mirror(
        &self,
        relay_url: &url::Url,
        source_url: &Url,
        blob_hash: &Sha256HexHash,
        signer: &BcrKeys,
    ) -> Result<Sha256HexHash>;
    /// Download the bytes with the given nostr_hash and compare if the hash matches the file
    async fn download(&self, relay_url: &url::Url, nostr_hash: &Sha256HexHash) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone, Default)]
pub struct FileStorageClient {
    cl: reqwest::Client,
}

impl ServiceTraitBounds for FileStorageClient {}

#[cfg(test)]
impl ServiceTraitBounds for MockFileStorageClientApi {}

impl FileStorageClient {
    pub fn new() -> Self {
        Self {
            cl: reqwest::Client::new(),
        }
    }
}

pub fn normalize_storage_base_url(url: &url::Url) -> Result<Url> {
    let mut normalized = url.clone();
    match normalized.scheme() {
        "ws" => {
            normalized
                .set_scheme("http")
                .map_err(|_| Error::InvalidRelayUrl)?;
        }
        "wss" => {
            normalized
                .set_scheme("https")
                .map_err(|_| Error::InvalidRelayUrl)?;
        }
        _ => (),
    };
    Ok(normalized)
}

pub fn to_url(relay_url: &url::Url, to_join: &str) -> Result<Url> {
    let normalized = normalize_storage_base_url(relay_url)?;
    Ok(normalized
        .join(to_join)
        .map_err(|_| Error::InvalidRelayUrl)?)
}

fn sha256_hash(bytes: &[u8]) -> Result<Sha256HexHash> {
    let mut hash_engine = sha256::HashEngine::default();
    hash_engine.write_all(bytes).map_err(|_| Error::Hash)?;
    Ok(sha256::Hash::from_engine(hash_engine))
}

fn blossom_auth_header(signer: &BcrKeys, blob_hash: &Sha256HexHash) -> Result<String> {
    let expiration = Timestamp::from_secs(Timestamp::now().as_secs() + 60);
    let event = EventBuilder::new(Kind::Custom(24242), "")
        .tags([
            Tag::parse(["t", "upload"]).map_err(|err| Error::Auth(err.to_string()))?,
            Tag::parse(["x", blob_hash.to_string().as_str()])
                .map_err(|err| Error::Auth(err.to_string()))?,
            Tag::parse(["expiration", expiration.as_secs().to_string().as_str()])
                .map_err(|err| Error::Auth(err.to_string()))?,
        ])
        .finalize(&signer.get_nostr_keys())
        .map_err(|err| Error::Auth(err.to_string()))?;

    Ok(format!("Nostr {}", URL_SAFE_NO_PAD.encode(event.as_json())))
}

#[derive(Debug, Clone, Serialize)]
struct MirrorRequest<'a> {
    url: &'a str,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl FileStorageClientApi for FileStorageClient {
    async fn upload(&self, relay_url: &url::Url, bytes: Vec<u8>) -> Result<Sha256HexHash> {
        let hash = sha256_hash(&bytes)?;

        // Make upload request
        let resp: BlobDescriptorReply = self
            .cl
            .put(to_url(relay_url, "upload")?)
            .body(bytes)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let nostr_hash = resp.sha256;

        // Check hash
        if hash != nostr_hash {
            return Err(Error::InvalidHash.into());
        }

        Ok(nostr_hash)
    }

    async fn mirror(
        &self,
        relay_url: &url::Url,
        source_url: &Url,
        blob_hash: &Sha256HexHash,
        signer: &BcrKeys,
    ) -> Result<Sha256HexHash> {
        let auth_header = blossom_auth_header(signer, blob_hash)?;

        let resp: BlobDescriptorReply = self
            .cl
            .put(to_url(relay_url, "mirror")?)
            .header("Authorization", auth_header)
            .json(&MirrorRequest {
                url: source_url.as_str(),
            })
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        if *blob_hash != resp.sha256 {
            return Err(Error::InvalidHash.into());
        }

        Ok(resp.sha256)
    }

    async fn download(&self, relay_url: &url::Url, nostr_hash: &Sha256HexHash) -> Result<Vec<u8>> {
        // Make download request
        let resp: Vec<u8> = self
            .cl
            .get(to_url(relay_url, &nostr_hash.to_string())?)
            .send()
            .await?
            .bytes()
            .await?
            .into();

        // Calculate hash to compare with the hash we sent
        let mut hash_engine = sha256::HashEngine::default();
        if hash_engine.write_all(&resp).is_err() {
            return Err(Error::Hash.into());
        }

        // Check hash
        let hash = sha256::Hash::from_engine(hash_engine);
        if &hash != nostr_hash {
            return Err(Error::InvalidHash.into());
        }

        Ok(resp)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlobDescriptorReply {
    sha256: Sha256HexHash,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::event::Event;
    use std::str::FromStr;

    #[test]
    fn normalize_storage_base_url_converts_websocket_schemes() {
        assert_eq!(
            normalize_storage_base_url(&url::Url::parse("ws://relay.example.com").unwrap())
                .unwrap()
                .as_str(),
            "http://relay.example.com/"
        );
        assert_eq!(
            normalize_storage_base_url(&url::Url::parse("wss://relay.example.com").unwrap())
                .unwrap()
                .as_str(),
            "https://relay.example.com/"
        );
        assert_eq!(
            normalize_storage_base_url(&url::Url::parse("https://relay.example.com").unwrap())
                .unwrap()
                .as_str(),
            "https://relay.example.com/"
        );
    }

    #[test]
    fn blossom_auth_header_uses_url_safe_base64() {
        let signer = BcrKeys::new();
        let hash = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();

        let header = blossom_auth_header(&signer, &hash).unwrap();
        let encoded = header.strip_prefix("Nostr ").unwrap();
        let event_json = URL_SAFE_NO_PAD.decode(encoded).unwrap();
        let event = Event::from_json(event_json).unwrap();

        assert_eq!(event.kind, Kind::Custom(24242));
        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| tag.kind() == "x")
                .and_then(|tag| tag.content()),
            Some(hash.to_string().as_str())
        );
    }

    #[test]
    fn mirror_request_serializes_url_field() {
        let request = MirrorRequest {
            url: "https://example.com/blob",
        };

        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"url":"https://example.com/blob"}"#
        );
    }
}
