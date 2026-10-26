//! Single-password authentication for the default Redis user.
//!
//! This is a deliberately narrow step toward production security: one operator
//! configured password, constant-time digest comparison and `NOAUTH` rejection
//! before data access. Multi-user ACLs, per-command permissions and TLS remain
//! separate gates documented in `docs/security.md`.

use super::sha256;

/// Credential digest for the implicit `default` user.
#[derive(Clone, Debug, Default)]
pub struct Auth {
    password: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// No password is configured; Redis reports the misconfigured AUTH call.
    NoPassword,
    /// Unknown user, wrong password or a disabled default credential.
    WrongPassword,
    /// A poisoned credential lock; fail closed rather than allow access.
    Internal,
}

impl Auth {
    /// An empty password explicitly disables authentication, matching
    /// `CONFIG SET requirepass ""`. `None` also disables it.
    pub fn from_password(password: Option<&[u8]>) -> Self {
        match password {
            Some(password) if !password.is_empty() => Self {
                password: Some(sha256::digest(password)),
            },
            _ => Self { password: None },
        }
    }

    pub fn required(&self) -> bool {
        self.password.is_some()
    }

    pub fn configured(&self) -> bool {
        self.password.is_some()
    }

    pub fn set_password(&mut self, password: Option<&[u8]>) {
        *self = Self::from_password(password);
    }

    /// Verify the default user's password. User selection is validated even
    /// though only `default` exists, so a future ACL implementation can extend
    /// the same call site without silently accepting arbitrary user names.
    pub fn verify(&self, username: Option<&[u8]>, password: &[u8]) -> Result<(), AuthError> {
        let Some(stored) = self.password else {
            return Err(AuthError::NoPassword);
        };
        if username.is_some_and(|username| !username.eq_ignore_ascii_case(b"default")) {
            return Err(AuthError::WrongPassword);
        }
        if sha256::constant_time_eq(&stored, &sha256::digest(password)) {
            Ok(())
        } else {
            Err(AuthError::WrongPassword)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_configured_default_credentials_authenticate() {
        let auth = Auth::from_password(Some(b"correct horse"));
        assert!(auth.required());
        assert_eq!(auth.verify(None, b"correct horse"), Ok(()));
        assert_eq!(auth.verify(Some(b"default"), b"correct horse"), Ok(()));
        assert_eq!(auth.verify(Some(b"DEFAULT"), b"correct horse"), Ok(()));
        assert_eq!(
            auth.verify(Some(b"other"), b"correct horse"),
            Err(AuthError::WrongPassword)
        );
        assert_eq!(auth.verify(None, b"wrong"), Err(AuthError::WrongPassword));
        assert_eq!(auth.verify(None, b""), Err(AuthError::WrongPassword));
    }

    #[test]
    fn disabled_and_rotated_passwords_follow_explicit_state() {
        let mut auth = Auth::from_password(None);
        assert!(!auth.required());
        assert_eq!(auth.verify(None, b""), Err(AuthError::NoPassword));
        auth.set_password(Some(b"first"));
        assert_eq!(auth.verify(None, b"first"), Ok(()));
        auth.set_password(Some(b"second"));
        assert_eq!(auth.verify(None, b"first"), Err(AuthError::WrongPassword));
        assert_eq!(auth.verify(None, b"second"), Ok(()));
        auth.set_password(None);
        assert!(!auth.required());
        assert!(!auth.configured());
        assert_eq!(auth.verify(None, b"second"), Err(AuthError::NoPassword));
    }
}
