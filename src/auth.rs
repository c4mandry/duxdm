//! PAM authentication and session management. duxdm ships a
//! `/etc/pam.d/duxdm` config that includes the distro's `login` stack.

use pam::{Client, PasswordConv};

/// Verify a username/password against PAM and open a PAM session.
///
/// The returned Client must be kept alive for the whole user session —
/// dropping it closes the PAM session (`close_on_drop` is on by default).
pub fn authenticate(
    username: &str,
    password: &str,
) -> Result<Client<'static, PasswordConv>, String> {
    let mut client =
        Client::with_password("duxdm").map_err(|e| format!("PAM init failed: {e}"))?;
    client.conversation_mut().set_credentials(username, password);
    client
        .authenticate()
        .map_err(|e| format!("authentication failed: {e}"))?;
    client
        .open_session()
        .map_err(|e| format!("could not open session: {e}"))?;
    Ok(client)
}
