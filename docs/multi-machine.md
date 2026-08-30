# Multi-machine usage

QuotaStation can combine usage from two or more Windows computers. Each computer reads its
own local session files and writes hourly and daily totals to a folder managed by a file-sync
tool. Every computer can then show the combined history or filter it by device.

Live quota is not shared in the same way. A computer can show current quota only when the
provider's client supplies it on that computer. Usage imported from another device still
appears in charts, model totals, and cost estimates, but it does not create a live quota
reading.

## What is shared

Each device writes one JSON file containing:

- a randomly generated device ID and the display name chosen in Settings;
- the time zone and parser version used to calculate the totals;
- hourly and daily token totals grouped by provider and model;
- the estimated API cost for each row; and
- confirmed quota reset events.

The file never contains project names, local paths, session IDs, prompts, account details,
credentials, source code, or raw provider logs.

Each computer writes only its own file and reads the files written by the others. There is no
QuotaStation server or primary computer, and two devices never edit the same file. A file made
in a different time zone is skipped because its hourly rows describe different local hours.

Reset events describe the provider account rather than one device, so every computer merges
them into one history. The estimated token total for each reset window is recalculated from
the usage available on the computer reading the file.

## Files created by sync tools

A sync tool may create a conflict copy when a file changes during upload. QuotaStation reads
only names in the exact form `usage-<device id>.json`, so conflict copies do not change the
totals or cause an error. They are stale duplicates and can be deleted safely.

Install QuotaStation separately on each computer. Do not copy its application-data directory
between them. The device ID is created on first start; copying it would make two computers
overwrite the same shared file.

## Configure QuotaStation

On each computer:

1. Open **Settings** and find **Shared usage folder**.
2. Enter a name that identifies the computer.
3. Choose the local folder managed by the sync tool. The local path can be different on each
   computer.
4. Refresh QuotaStation after the folder exists. Export and import then run automatically
   after each history refresh.

Clearing the folder setting disables sharing. Usage already imported into the local database
is left in place.

## Choose one sync method

Use one sync tool for the shared folder:

- **Proton Drive** works well when the computers are rarely online together. One can upload a
  change and shut down before the other downloads it.
- **Syncthing** transfers directly between computers and does not keep a central cloud copy.
  The computers must be online at the same time occasionally.

Both services use their own account or device identity, independent of the Windows account.

## Option A: Proton Drive

1. Download the Windows app from the
   [official Proton Drive page](https://proton.me/drive/download) and install it on each
   computer.
2. Sign in to the same Proton Account on each computer.
3. In File Explorer, create a `QuotaStation` folder under `Proton Drive\My files` and wait for
   it to appear on the other computers.
4. Right-click the folder on each computer and choose **Always keep on this device**. This
   prevents an online-only placeholder from hiding a file from QuotaStation.
5. Select the local `QuotaStation` folder in QuotaStation's **Shared usage folder** setting.
6. Leave Proton Drive's start-with-Windows setting enabled so pending changes transfer the
   next time each computer starts.

Proton Drive encrypts file contents and names end to end. The computers do not need to be
online together. A managed computer may still require IT approval before Proton Drive can be
installed.

## Option B: Syncthing

Download the base Windows build from the
[official Syncthing downloads page](https://syncthing.net/downloads/). It is a portable
archive: extract it to a permanent folder and run `syncthing.exe`. It does not require
administrator access.

1. Start Syncthing once on each computer and open its local web interface.
2. On one computer, copy its device ID from **Actions > Show ID** and add it as a remote device
   on the other. Repeat this for every pair of computers that should connect.
3. Add a new empty folder on one computer and share it with the other devices.
4. Accept the folder on each device and choose its local path.
5. Wait until Syncthing reports the folder as up to date, then select each local path in
   QuotaStation's **Shared usage folder** setting.

Syncthing sends files directly between the devices. Discovery and relay services help them
connect, but relay traffic remains encrypted between the devices. No server, port forwarding,
static IP address, or private relay is normally required. If a managed firewall blocks
Syncthing, allow the Windows firewall prompt or ask the network administrator.

To start the portable build automatically without administrator access, create a shortcut in:

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup
```

Set its target to the extracted executable followed by:

```text
C:\path\to\syncthing.exe --no-console --no-browser
```

This starts Syncthing at user login without opening a console or browser. The
[official Syncthing autostart guide](https://docs.syncthing.net/users/autostart.html#run-at-user-log-on-using-the-startup-folder)
also documents the `shell:startup` shortcut.

## Do not combine sync tools

Do not let two sync tools manage the same folder. In particular:

- do not add the Proton Drive folder to Syncthing; and
- do not place the Syncthing folder inside Proton Drive, OneDrive, Dropbox, Google Drive, or
  another synced folder.

Two tools writing the same files can race and create unnecessary conflict copies.
