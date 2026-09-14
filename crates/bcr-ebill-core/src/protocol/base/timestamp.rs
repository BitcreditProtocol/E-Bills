use crate::protocol::{DateTimeUtc, ProtocolValidationError};
use serde::{Deserialize, Serialize};
use std::{
    fmt::Display,
    ops::{Add, Sub},
    time::Duration,
};
use time::Time;

#[derive(Copy, Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, Hash)]
#[serde(try_from = "u64", into = "u64")]
pub struct Timestamp(u64);

impl Timestamp {
    pub fn new(timestamp: u64) -> Result<Timestamp, ProtocolValidationError> {
        let timestamp_i64 =
            i64::try_from(timestamp).map_err(|_| ProtocolValidationError::InvalidTimestamp)?;

        DateTimeUtc::from_unix_timestamp(timestamp_i64)
            .map_err(|_| ProtocolValidationError::InvalidTimestamp)?;

        Ok(Timestamp(timestamp))
    }

    pub fn now() -> Self {
        Timestamp(DateTimeUtc::now_utc().unix_timestamp() as u64)
    }

    pub fn zero() -> Self {
        Timestamp(0)
    }

    pub fn inner(&self) -> u64 {
        self.0
    }

    pub fn start_of_day(&self) -> Timestamp {
        let dt = self.to_datetime();
        Timestamp(dt.replace_time(Time::MIDNIGHT).unix_timestamp() as u64)
    }

    pub fn end_of_day(&self) -> Timestamp {
        let dt = self.to_datetime();
        let end_of_day = Time::from_hms(23, 59, 59).expect("valid time");

        Timestamp(dt.replace_time(end_of_day).unix_timestamp() as u64)
    }

    pub fn to_datetime(&self) -> DateTimeUtc {
        // safe, since we check bounds during creation
        DateTimeUtc::from_unix_timestamp(self.0 as i64).expect("invalid timestamp")
    }

    pub fn has_deadline_passed(&self, deadline: &Timestamp) -> bool {
        self > deadline
    }

    pub fn deadline_is_at_or_after_end_of_day_of(&self, deadline_timestamp: &Timestamp) -> bool {
        let end_of_day_base_ts = &self.end_of_day();
        deadline_timestamp >= end_of_day_base_ts
    }
}

impl borsh::BorshSerialize for Timestamp {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        borsh::BorshSerialize::serialize(&self.0, writer)
    }
}

impl borsh::BorshDeserialize for Timestamp {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let ts_u64: u64 = borsh::BorshDeserialize::deserialize_reader(reader)?;
        Timestamp::new(ts_u64).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

impl Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<DateTimeUtc> for Timestamp {
    fn from(value: DateTimeUtc) -> Self {
        Timestamp(value.unix_timestamp() as u64)
    }
}

impl From<nostr::types::Timestamp> for Timestamp {
    fn from(value: nostr::types::Timestamp) -> Self {
        Timestamp(value.as_secs())
    }
}

impl From<Timestamp> for nostr::types::Timestamp {
    fn from(value: Timestamp) -> nostr::types::Timestamp {
        nostr::types::Timestamp::from_secs(value.0)
    }
}

impl Add<Timestamp> for Timestamp {
    type Output = Self;
    fn add(self, rhs: Timestamp) -> Self::Output {
        Timestamp(self.0.saturating_add(rhs.inner()))
    }
}

impl Sub<Timestamp> for Timestamp {
    type Output = Self;
    fn sub(self, rhs: Timestamp) -> Self::Output {
        Timestamp(self.0.saturating_sub(rhs.inner()))
    }
}

impl Add<Duration> for Timestamp {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        Timestamp(self.0.saturating_add(rhs.as_secs()))
    }
}

impl Sub<Duration> for Timestamp {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        Timestamp(self.0.saturating_sub(rhs.as_secs()))
    }
}

impl Add<u64> for Timestamp {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        Timestamp(self.0.saturating_add(rhs))
    }
}

impl Sub<u64> for Timestamp {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self::Output {
        Timestamp(self.0.saturating_sub(rhs))
    }
}

impl TryFrom<u64> for Timestamp {
    type Error = ProtocolValidationError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Timestamp::new(value)
    }
}

impl From<Timestamp> for u64 {
    fn from(value: Timestamp) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::tests::tests::test_ts;
    use borsh::BorshDeserialize;
    use serde::{Deserialize, Serialize};
    use std::time::UNIX_EPOCH;
    use time::macros::datetime;

    #[derive(
        Debug,
        Clone,
        Eq,
        PartialEq,
        borsh_derive::BorshSerialize,
        borsh_derive::BorshDeserialize,
        Serialize,
        Deserialize,
    )]
    pub struct TestTimestamp {
        pub ts: Timestamp,
    }

    #[test]
    fn test_serialization() {
        let ts = Timestamp::new(1731593928).expect("works");
        let test = TestTimestamp { ts };
        let json = serde_json::to_string(&test).unwrap();
        assert_eq!("{\"ts\":1731593928}", json);
        let deserialized = serde_json::from_str(&json).unwrap();
        assert_eq!(test, deserialized);
        assert_eq!(ts, deserialized.ts);

        let borsh = borsh::to_vec(&ts).unwrap();
        let borsh_de = Timestamp::try_from_slice(&borsh).unwrap();
        assert_eq!(ts, borsh_de);

        let borsh_test = borsh::to_vec(&test).unwrap();
        let borsh_de_test = TestTimestamp::try_from_slice(&borsh_test).unwrap();
        assert_eq!(test, borsh_de_test);
        assert_eq!(ts, borsh_de_test.ts);
    }

    #[test]
    fn test_timestamp() {
        let n = test_ts();
        let n_owned = test_ts();
        let n_bigger = n_owned + 1;
        assert_eq!(n, n_owned);
        assert!(n < n_bigger);

        assert!(matches!(
            Timestamp::new(u64::MAX),
            Err(ProtocolValidationError::InvalidTimestamp)
        ));
    }

    #[test]
    fn test_invalid_serialization() {
        let json = "{\"timestamp\":-5}";
        let deserialized = serde_json::from_str::<TestTimestamp>(json);
        assert!(deserialized.is_err());

        let bytes = 5u64.to_le_bytes().to_vec();
        let res = Timestamp::try_from_slice(&bytes).expect("works");
        assert_eq!(res, Timestamp(5));

        let borsh = borsh::to_vec(&String::from("invalid")).expect("works");
        let res = Timestamp::try_from_slice(&borsh);
        assert!(res.is_err());
    }

    #[test]
    fn test_now() {
        let now = Timestamp::now();
        let timestamp = Timestamp::new(
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                - 1,
        )
        .expect("works");
        assert!(
            now >= timestamp - 1,
            "now date was {} seconds smaller than expected",
            (timestamp - now)
        );
    }

    #[test]
    fn test_start_of_day() {
        let ts: Timestamp = datetime!(2025-01-15 5:10:45 UTC).into();
        let start_of_day = ts.start_of_day();
        assert!(start_of_day < ts,);
    }

    #[test]
    fn test_end_of_day() {
        let ts: Timestamp = datetime!(2025-01-15 0:00:00 UTC).into();
        let end_of_day = ts.end_of_day();
        assert!(end_of_day > ts,);
        let end_of_day_end_of_dayd = ts.end_of_day();
        assert_eq!(end_of_day, end_of_day_end_of_dayd);
    }

    #[test]
    fn test_deadline_is_at_or_after_end_of_day_of() {
        let ts: Timestamp = datetime!(2025-01-15 0:00:00 UTC).into();
        let end_of_day = ts.end_of_day();
        assert!(ts.deadline_is_at_or_after_end_of_day_of(&end_of_day));
        assert!(!ts.deadline_is_at_or_after_end_of_day_of(&(end_of_day - 1)));
        assert!(ts.deadline_is_at_or_after_end_of_day_of(&(end_of_day + 1)));
    }

    #[test]
    fn test_validate_timestamp() {
        assert!(Timestamp::new(0).is_ok());
        assert!(Timestamp::new(1731593928).is_ok());
        assert!(Timestamp::new(u64::MAX).is_err());
    }
}
