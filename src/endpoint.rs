use log::{trace, warn};
use nix::unistd;
use std::{
    fs::{set_permissions, Permissions},
    os::unix::fs::{self, PermissionsExt},
    path::PathBuf,
};

type Result = std::result::Result<(), String>;

#[derive(Debug)]
pub enum Endpoint {
    Inet(String),
    Unix(UnixDomainSocket),
}

#[derive(Debug, Default)]
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

    pub(crate) fn remove_file(&self) {
        let path = self.path.as_path();

        match std::fs::remove_file(path) {
            Ok(()) => {
                trace!("Removed Unix domain socket file '{}'", path.display())
            }
            Err(err) => warn!(
                "Failed to remove Unix domain socket file '{}': {err}",
                path.display()
            ),
        }
    }

    pub(crate) fn set_permissions(&self) -> Result {
        self.chown()?;
        self.chmod()?;

        Ok(())
    }
}
