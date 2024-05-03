use nix::unistd;
#[cfg(feature = "serde")]
use serde::Deserialize;
use std::{
    convert::Infallible,
    fs::{set_permissions, Permissions},
    os::unix::fs::{self, PermissionsExt},
    path::PathBuf,
    result,
    str::FromStr,
};

type Result = result::Result<(), String>;

#[derive(Debug)]
pub enum Endpoint {
    Inet(String),
    Unix(UnixDomainSocket),
}

impl FromStr for Endpoint {
    type Err = Infallible;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        if s.starts_with('/') {
            Ok(Self::Unix(s.parse().unwrap()))
        } else {
            Ok(Self::Inet(s.to_string()))
        }
    }
}

#[derive(Debug, Default)]
#[cfg_attr(feature = "serde", derive(Deserialize))]
pub struct UnixDomainSocket {
    pub path: PathBuf,
    pub mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

impl UnixDomainSocket {
    fn chmod(&self) -> Result {
        if let Some(permissions) = self.mode.map(Permissions::from_mode) {
            set_permissions(&self.path, permissions).map_err(|err| {
                format!(
                    "Failed to set permissions for '{}': {err}",
                    self.path.display()
                )
            })?;
        }

        Ok(())
    }

    fn chown(&self) -> Result {
        let owner = match self.owner {
            Some(ref value) => match value.parse::<u32>().ok() {
                Some(id) => Some(id),
                None => match unistd::User::from_name(value) {
                    Ok(user) => match user {
                        Some(user) => Some(user.uid.as_raw()),
                        None => {
                            return Err(format!("user '{value}' not found"))
                        }
                    },
                    Err(err) => {
                        return Err(format!(
                            "Failed to find user named '{value}': {err}"
                        ))
                    }
                },
            },
            None => None,
        };

        let group = match self.group {
            Some(ref value) => match value.parse::<u32>().ok() {
                Some(id) => Some(id),
                None => match unistd::Group::from_name(value) {
                    Ok(group) => match group {
                        Some(group) => Some(group.gid.as_raw()),
                        None => {
                            return Err(format!("group '{value}' not found"))
                        }
                    },
                    Err(err) => {
                        return Err(format!(
                            "Failed to find group named '{value}': {err}"
                        ))
                    }
                },
            },
            None => None,
        };

        fs::chown(&self.path, owner, group).map_err(|err| {
            format!(
                "Failed to change owner and group of '{}': {err}",
                self.path.display()
            )
        })
    }

    pub(crate) fn set_permissions(&self) -> Result {
        self.chown()?;
        self.chmod()?;

        Ok(())
    }
}

impl FromStr for UnixDomainSocket {
    type Err = Infallible;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        Ok(Self {
            path: PathBuf::from(s),
            ..Default::default()
        })
    }
}
