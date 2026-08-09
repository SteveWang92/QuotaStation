# Security policy

## Supported versions

QuotaStation has not published a release yet. Supported-version information will be added
with the first public release.

## Reporting a vulnerability

Please report security issues privately to **contact@stevewang.me**. Do not open a public
issue for a suspected credential leak, local data exposure, unsafe provider integration, or
dependency vulnerability with a working exploit.

Include a concise description, affected component, reproduction conditions, and impact.
Remove credentials, account identifiers, prompts, source code, and private paths from the
report. You can expect an acknowledgement within seven days.

## Security boundary

QuotaStation is intended to read usage and entitlement information without modifying
provider accounts. Credential material must remain in the provider's client or operating
system credential store and must not be written to QuotaStation history or diagnostics.
