#![cfg(feature = "serde")]

use crate::{Endpoint, UnixDomainSocket};

use serde::{
    de::{Deserialize, Error, MapAccess, Visitor},
    ser::{Serialize, SerializeMap, Serializer},
};
use std::{fmt, path::PathBuf};

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
                write!(
                    formatter,
                    "a path or map of options for a Unix domain socket"
                )
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut uds = UnixDomainSocket::default();

                while let Some((key, value)) = map.next_entry::<&str, &str>()? {
                    match key {
                        "path" => uds.path = PathBuf::from(value),
                        "mode" => {
                            uds.mode = Some(
                                u32::from_str_radix(value, 8)
                                    .map_err(Error::custom)?,
                            )
                        }
                        "owner" => {
                            uds.owner =
                                Some(value.parse().map_err(Error::custom)?)
                        }
                        "group" => {
                            uds.group =
                                Some(value.parse().map_err(Error::custom)?)
                        }
                        _ => {
                            return Err(Error::unknown_field(
                                key,
                                &["path", "mode", "owner", "group"],
                            ))
                        }
                    }
                }

                Ok(Endpoint::Unix(uds))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                if value.starts_with('/') {
                    Ok(Endpoint::Unix(UnixDomainSocket {
                        path: PathBuf::from(value),
                        ..Default::default()
                    }))
                } else {
                    Ok(Endpoint::Inet(value.into()))
                }
            }
        }

        deserializer.deserialize_any(EndpointVisitor)
    }
}
