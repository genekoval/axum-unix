#![cfg(feature = "serde")]

use crate::Endpoint;

use serde::{
    de::{
        value::MapAccessDeserializer, Deserialize, Error, MapAccess, Visitor,
    },
    ser::{Serialize, SerializeMap, Serializer},
};
use std::fmt;

impl Serialize for Endpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Endpoint::Inet(inet) => serializer.serialize_str(inet),
            Endpoint::Unix(uds) => {
                if uds.mode.is_none()
                    && uds.owner.is_none()
                    && uds.group.is_none()
                {
                    serializer.serialize_str(uds.path.to_str().unwrap())
                } else {
                    let mut map = serializer.serialize_map(None)?;

                    map.serialize_entry("path", uds.path.to_str().unwrap())?;

                    if let Some(mode) = uds.mode {
                        map.serialize_entry("mode", &mode)?;
                    }

                    if let Some(ref owner) = uds.owner {
                        map.serialize_entry("owner", owner)?;
                    }

                    if let Some(ref group) = uds.group {
                        map.serialize_entry("group", group)?;
                    }

                    map.end()
                }
            }
        }
    }
}

impl<'de> Deserialize<'de> for Endpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EndpointVisitor;

        impl<'de> Visitor<'de> for EndpointVisitor {
            type Value = Endpoint;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string or map")
            }

            fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let deserializer = MapAccessDeserializer::new(map);
                let uds = Deserialize::deserialize(deserializer)?;

                Ok(Self::Value::Unix(uds))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(value.parse().unwrap())
            }
        }

        deserializer.deserialize_any(EndpointVisitor)
    }
}
