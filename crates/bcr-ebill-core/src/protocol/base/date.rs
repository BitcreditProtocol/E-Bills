use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use time::{
    Date as TimeDate, format_description::StaticFormatDescription, macros::format_description,
};

use crate::protocol::{DateTimeUtc, ProtocolValidationError, Timestamp};

pub const DEFAULT_DATE_FORMAT: StaticFormatDescription =
    format_description!("[year]-[month]-[day]");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct Date(String);

impl Date {
    pub fn new(n: impl Into<String>) -> Result<Self, ProtocolValidationError> {
        let s = n.into();

        TimeDate::parse(&s, DEFAULT_DATE_FORMAT)
            .map_err(|_| ProtocolValidationError::InvalidDate)?;

        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_timestamp(&self) -> Timestamp {
        let date = TimeDate::parse(&self.0, DEFAULT_DATE_FORMAT).expect("has the right format");

        let timestamp = date.midnight().assume_utc().unix_timestamp();

        Timestamp::new(timestamp as u64).expect("checked")
    }
}

impl From<DateTimeUtc> for Date {
    fn from(value: DateTimeUtc) -> Self {
        Date(
            value
                .format(DEFAULT_DATE_FORMAT)
                .expect("date format is valid"),
        )
    }
}

impl From<Timestamp> for Date {
    fn from(value: Timestamp) -> Self {
        Date(
            value
                .to_datetime()
                .format(DEFAULT_DATE_FORMAT)
                .expect("date format is valid"),
        )
    }
}

impl Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Date {
    type Err = ProtocolValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Date::new(s)
    }
}

impl TryFrom<String> for Date {
    type Error = ProtocolValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Date> for String {
    fn from(value: Date) -> Self {
        value.0
    }
}

impl borsh::BorshSerialize for Date {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        borsh::BorshSerialize::serialize(&self.0, writer)
    }
}

impl borsh::BorshDeserialize for Date {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let date_str: String = borsh::BorshDeserialize::deserialize_reader(reader)?;
        Date::new(&date_str).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use borsh::BorshDeserialize;
    use serde::{Deserialize, Serialize};
    use time::Month;

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
    pub struct TestDate {
        pub date: Date,
    }

    #[test]
    fn test_serialization() {
        let date = Date::new("2025-01-15").expect("works");
        let test = TestDate { date: date.clone() };
        let json = serde_json::to_string(&test).unwrap();
        assert_eq!("{\"date\":\"2025-01-15\"}", json);
        let deserialized = serde_json::from_str(&json).unwrap();
        assert_eq!(test, deserialized);
        assert_eq!(date, deserialized.date);

        let borsh = borsh::to_vec(&date).unwrap();
        let borsh_de = Date::try_from_slice(&borsh).unwrap();
        assert_eq!(date, borsh_de);

        let borsh_test = borsh::to_vec(&test).unwrap();
        let borsh_de_test = TestDate::try_from_slice(&borsh_test).unwrap();
        assert_eq!(test, borsh_de_test);
        assert_eq!(date, borsh_de_test.date);
    }

    #[test]
    fn test_date() {
        let n = Date::new("2025-01-15").expect("works");
        let n_owned = Date::new(String::from("2025-01-15")).expect("works");
        assert_eq!(n, n_owned);

        assert!(matches!(
            Date::new("1234"),
            Err(ProtocolValidationError::InvalidDate)
        ));
        assert!(matches!(
            Date::new("01.03.2025"),
            Err(ProtocolValidationError::InvalidDate)
        ));
    }

    #[test]
    fn test_invalid_serialization() {
        let json = "{\"date\":\"invalid\"}";
        let deserialized = serde_json::from_str::<TestDate>(json);
        assert!(deserialized.is_err());

        let borsh = borsh::to_vec(&String::from("2025-01-15")).expect("works");
        let res = Date::try_from_slice(&borsh);
        assert!(res.is_ok());

        let borsh = borsh::to_vec(&String::from("invalid")).expect("works");
        let res = Date::try_from_slice(&borsh);
        assert!(res.is_err());
    }

    #[test]
    fn test_date_string_to_timestamp_with_default_format() {
        let date_str = "2025-01-15";
        let expected_timestamp: Timestamp = TimeDate::from_calendar_date(2025, Month::January, 15)
            .unwrap()
            .midnight()
            .assume_utc()
            .unix_timestamp()
            .try_into()
            .map(Timestamp::new)
            .unwrap()
            .unwrap();
        assert_eq!(
            Date::new(date_str).unwrap().to_timestamp(),
            expected_timestamp
        );
    }

    #[test]
    fn test_date_string_to_timestamp_with_invalid_date() {
        assert!(Date::new("2025-32-99").is_err());
        assert!(Date::new("2025/01/15").is_err());
        assert!(Date::new("").is_err());
    }
}
