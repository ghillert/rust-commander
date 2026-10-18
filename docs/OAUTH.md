# Browser OAuth setup

[← Remote connections](REMOTE_CONNECTIONS.md)

Open **File → Connections**, select Google Drive (`gdrive`), Dropbox (`dropbox`)
or OneDrive (`onedrive`), and enter the OAuth client ID for your own registered
application. Rust Commander does not ship a shared provider registration.

The default callback port is **53682**. Choose another unused port if necessary,
and match the registered redirect URI to the value displayed in Connections.
The listener accepts IPv4 loopback connections only and validates a random state
and PKCE challenge. Sign-in times out after three minutes; closing the dialog
cancels the pending browser authorization.

| Provider | Registration and callback | Requested access |
| --- | --- | --- |
| Google Drive | Enable Drive API and configure consent. Create a **Desktop app** OAuth client. Redirect: `http://127.0.0.1:53682/callback`. Enter the client secret if issued for that desktop client. | Drive file access; offline access with consent for a refresh token. |
| Dropbox | Create a scoped app in the App Console and enable file metadata/content read and write permissions. Register `http://127.0.0.1:53682/callback`. The app key is the client ID; PKCE supports public clients without an app secret. | Metadata and file read/write; offline token access. App-folder vs full-account access depends on registration. |
| OneDrive | Create an Entra application registration with the account types you need. Add a **Mobile and desktop applications** redirect: `http://localhost:53682/callback`. Use its application/client ID and tenant (`common` by default). Public desktop clients do not need a client secret. | Delegated `Files.ReadWrite` and `offline_access`. Tenant policy may require admin approval. |

Choose **Sign in with browser**, approve the requested access in the provider's
page and return to Rust Commander. The app exchanges the code for tokens and
connects the profile. Access and refresh tokens stay in memory unless **Remember
credentials** is selected; then they use the operating system credential store.
Client IDs, tenant and callback port are nonsecret profile settings. Tokens and
client secrets are never written into settings, queue history or browser results.

Before subsequent remote requests, expired OAuth sessions refresh and reconnect.
A rotated refresh token replaces its predecessor; an omitted refresh token
preserves the previous one. Reconnects are serialized to avoid simultaneous
refreshes spending a rotating token twice. Revoked or unavailable credentials
produce an error; use Connections to sign in again. Manually supplied access
tokens without refresh information still work but cannot renew automatically.

Protocol tests use loopback servers for token encoding, expiry/rotation handling,
HTTP fragmentation and error redaction. Live sign-in and provider/account policy
have not been tested with registered clients. Google consent screens in testing
mode may issue refresh tokens with a limited lifetime.

Provider documentation:

- [Google OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Dropbox OAuth guide](https://developers.dropbox.com/oauth-guide)
- [Dropbox offline OAuth access](https://dropbox.tech/developers/using-oauth-2-0-with-offline-access)
- [Microsoft redirect URI registration](https://learn.microsoft.com/en-us/entra/identity-platform/how-to-add-redirect-uri)
- [Microsoft redirect URI restrictions](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url)
