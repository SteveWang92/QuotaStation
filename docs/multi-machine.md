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

It also carries confirmed quota-window reset events. Resets are account-level facts, so every
machine merges the event set and shows one combined history; the per-window token total is
recalculated locally from the usage aggregates available on that machine.

The files never contain project names, local paths, session IDs, prompts, account details,
credentials, source code, or raw provider logs.

Each machine writes only its own file and reads the files written by the others. There is no
QuotaStation server, no primary machine, and no shared file that two devices both edit, so
QuotaStation itself has no merge conflict to resolve. A file aggregated in a different time
zone is refused rather than added to buckets that describe different local hours.

## Files the sync tool adds

A sync client sometimes writes a second copy of a file it was uploading when the file
changed again, naming it after the original with a marker such as `# Edit conflict … #`
added. QuotaStation ignores every name that is not exactly `usage-<device id>.json`, so such
a copy changes no total and reports no error. It is a stale duplicate of a file QuotaStation
rewrites in full on every refresh, and deleting it is safe at any time; nothing else in the
folder ever needs to be edited or merged by hand.

Give each machine its own installation rather than copying QuotaStation's application data
across. The device identity is written on first start, and two machines carrying the same
identity would overwrite one file instead of publishing two.

## Configure QuotaStation

On every machine:

1. Open **Settings** and find **Shared usage folder**.
2. Enter a display name for that machine.
3. Choose the corresponding local folder managed by the selected sync tool. The picker opens
   at the user's Documents folder when no folder has been chosen yet. The paths on the two
   machines do not need to be textually identical.
4. Refresh QuotaStation after the folder exists. Export and import then run automatically
   after history refreshes.

Leaving the folder path blank disables sharing without changing the usage already stored on
that machine.

## Choose a sync method

Use exactly one sync tool for the shared folder:

- **Proton Drive** is the recommended choice when the machines are rarely or never online
  together. Each machine can upload to Proton's cloud storage and shut down before the other
  downloads the change.
- **Syncthing** is the cloud-free choice when the machines regularly overlap online. It
  transfers directly between them, so they must be online at the same time at some point.

Both use accounts or device identities that are independent of the Windows sign-in account.

## Option A — Proton Drive

Proton Drive Free includes 5 GB and supports a Windows desktop application. QuotaStation's
aggregate files use only a tiny fraction of that allowance. Proton Drive encrypts file
contents and names end to end.

1. Download the app from the
   [official Proton Drive page](https://proton.me/drive/download) and install it on both
   machines.
2. Sign in to the same Proton Account on both machines. This account is separate from the
   Windows sign-in account.
3. In File Explorer, create a `QuotaStation` folder under
   `Proton Drive\My files`. Wait for it to appear on the other machine.
4. On both machines, right-click that folder and select **Always keep on this device**. This
   prevents an online-only placeholder from hiding an aggregate file from QuotaStation.
5. Select that local `QuotaStation` folder in QuotaStation's **Shared usage folder**
   setting on both machines.
6. Leave Proton Drive's start-with-Windows setting enabled so each machine uploads or
   downloads pending files whenever it next starts.

The machines do not need to overlap online. One can upload and shut down; Proton stores the
encrypted copy until the other machine next connects. Proton distributes a Windows installer,
so a managed computer whose policy blocks software installation may still require IT approval.

## Option B — Syncthing

Download Syncthing from its [official downloads page](https://syncthing.net/downloads/). The
base Windows build is a portable archive: extract it into a folder where it can stay and run
`syncthing.exe`. This does not require an installer or administrator access.

1. Start Syncthing once on both machines and open its local web interface.
2. On one machine, copy its device ID from **Actions > Show ID**. Add that ID as a remote
   device on the other machine, then repeat in the opposite direction.
3. Add a new empty folder on one machine and share it with the other device. Accept that
   folder on the second machine and choose a local path there.
4. Wait until Syncthing reports the folder as up to date, then enter each local path in
   QuotaStation's **Shared usage folder** setting.

Syncthing does not store the folder on a central server. The two machines exchange files
directly, and public discovery and relay services only help them find and reach each other.
Relay traffic remains end-to-end encrypted. Both machines do not need to stay online all the
time, but they must be online at the same time at some point for pending changes to transfer;
an offline machine catches up the next time they overlap.

No server, port forwarding, static IP address, or private relay is normally required. Device
IDs must be exchanged and the folder accepted on both machines as described above. Global
discovery, NAT traversal, and relaying are enabled by default; if a managed firewall blocks
Syncthing, allow the Windows firewall prompt or ask the network administrator to permit it.

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

## Do not layer sync tools

Do not let two sync clients manage the same shared folder. In particular:

- do not add the Proton Drive folder to Syncthing; and
- do not put the Syncthing folder inside Proton Drive, OneDrive, Dropbox, Google Drive, or
  another cloud-sync folder.

Two sync engines watching the same files can race and create conflicting copies.
