# Multi-machine usage

QuotaStation can combine usage parsed on two or more Windows machines. Each machine still
reads its own provider session logs, then exchanges only normalized aggregates through a
folder that a file-sync tool keeps in step. The dashboard combines every device by default
and can filter the whole usage panel to one device.

Live quota is different from usage. A provider's quota can be read only on a machine where
that provider's local client exposes it. A provider contributed only by another device still
has usage, charts, model totals, and costs on this machine, but its quota remains available
only on the source device.

## What is shared

Each device writes one JSON file containing only:

- the stable device ID and its display name;
- the aggregation time zone and parser revision;
- hourly and daily token totals, grouped by provider and model; and
- the API-equivalent cost attached to each aggregate row.

The files never contain project names, local paths, session IDs, prompts, account details,
credentials, source code, or raw provider logs.

Each machine writes only its own file and reads the files written by the others. There is no
QuotaStation server, no primary machine, and no shared file that two devices both edit, so
QuotaStation itself has no merge conflict to resolve. A file aggregated in a different time
zone is refused rather than added to buckets that describe different local hours.

## Configure QuotaStation

On every machine:

1. Open **Settings** and find **Shared usage folder**.
2. Enter a display name for that machine.
3. Enter the same synced folder path on that machine. The paths do not need to be textually
   identical; they only need to refer to the corresponding Syncthing folder.
4. Refresh QuotaStation after the folder exists. Export and import then run automatically
   after history refreshes.

Leaving the folder path blank disables sharing without changing the usage already stored on
that machine.

## Set up Syncthing on Windows

The official Windows build is a portable archive: extract it into a folder where it can stay
and run `syncthing.exe`. This does not require an installer or administrator access.

1. Start Syncthing once on both machines and open its local web interface.
2. On one machine, copy its device ID from **Actions > Show ID**. Add that ID as a remote
   device on the other machine, then repeat in the opposite direction.
3. Add a new empty folder on one machine and share it with the other device. Accept that
   folder on the second machine and choose a local path there.
4. Wait until Syncthing reports the folder as up to date, then enter each local path in
   QuotaStation's **Shared usage folder** setting.

Do not put this folder inside an existing OneDrive, Dropbox, Google Drive, or other cloud-sync
folder. Two sync engines watching the same files create a second synchronization layer and
can race or duplicate conflict handling.

For automatic startup without administrator access, create a shortcut in:

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup
```

Set the shortcut target to the extracted executable followed by the documented background
arguments:

```text
C:\path\to\syncthing.exe --no-console --no-browser
```

This starts Syncthing at user logon without opening a console or browser. The procedure and
the alternative `shell:startup` path are maintained in the
[official Syncthing autostart documentation](https://docs.syncthing.net/users/autostart.html#run-at-user-log-on-using-the-startup-folder).
