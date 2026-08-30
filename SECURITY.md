# Security policy

## Supported versions

Only the most recent released version receives fixes. Released versions are the `vX.Y.Z`
tags on `main`, listed on the [releases page](https://github.com/SteveWang92/QuotaStation/releases);
older versions are not patched, so upgrade before reporting a problem with one of them.

## Reporting a vulnerability

Please report security issues privately to **contact@stevewang.me**. Do not open a public
issue for a suspected credential leak, local data exposure, unsafe provider integration, or
dependency vulnerability with a working exploit.

Include a concise description, affected component, reproduction conditions, and impact.
Remove credentials, account identifiers, prompts, source code, and private paths from the
report. You can expect an acknowledgement within seven days.

## What QuotaStation can access

QuotaStation reads usage and quota information without changing provider accounts.
Credentials stay in the provider client or operating-system credential store and are never
written to QuotaStation history or diagnostics.
