# roedy-jmap

A minimal JMAP client (RFC 8620, RFC 8621) for the servers of the Roedy apps:
the session, requests with several method calls and back-references, and
sending a message (`Identity/get`, `Mailbox/get`, `Email/set` and
`EmailSubmission/set` with references to created objects). Roedy Hub sends
its system mails with it.

- The caller passes the HTTP client (`reqwest`, without default features).
  It must not follow redirects itself
  (`.redirect(reqwest::redirect::Policy::none())`): the crate follows them,
  only on the server's origin (three at most), so that credentials only go
  there – redirects of the session and the `apiUrl` must stay on it. With
  reqwest's default policy a redirect to another origin would be followed
  before the crate could check it.
- Responses are read up to 256 KiB at most.
- Errors and `Debug` never contain passwords or tokens.

JMAP in the browser is the npm package `@roedy/jmap` from the same
repository.

## Versions

Published with every Roedy Hub release under the hub's version (`0.x` until
the first release).

## Licence

AGPL-3.0-only, like Roedy Hub.
