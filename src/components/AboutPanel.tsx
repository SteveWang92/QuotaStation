import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { errorMessage } from "../errors";

/**
 * What the AGPL calls Appropriate Legal Notices: the program's name and version, its
 * copyright, the license it is offered under, the absence of a warranty, and where the
 * corresponding source is. Section 5(d) requires an interactive program to display these,
 * and a desktop application has nowhere to display them but a surface like this one.
 *
 * The links are plain text rather than anchors: no window here is allowed to navigate away
 * from the interface, and QuotaStation ships no shell-opener plugin, so an anchor would
 * simply do nothing when clicked. An address that can be read and copied satisfies the
 * requirement; one that silently fails does not.
 */
export function AboutPanel({
  appVersion,
  buildCommit,
}: {
  appVersion: string;
  buildCommit: string;
}) {
  const [openError, setOpenError] = useState<string | null>(null);

  const open = async (command: "open_data_folder" | "open_latest_release") => {
    setOpenError(null);
    try {
      await invoke(command);
    } catch (cause) {
      setOpenError(errorMessage(cause));
    }
  };

  return (
    <section className="provider-consent" aria-label="About QuotaStation">
      <div className="provider-consent-body">
        <h2>
          QuotaStation {appVersion} <code>{buildCommit}</code>
        </h2>
        <p>Copyright © 2026 Steve Wang</p>
        <p>
          Licensed under the GNU Affero General Public License, version 3 only. QuotaStation is free
          software: you may redistribute and modify it under those terms. It comes with absolutely
          no warranty, to the extent permitted by law.
        </p>
        <p>
          Source code and the full license text: <code>github.com/SteveWang92/QuotaStation</code>
        </p>
        <p>
          The reused open-source components, their pinned revisions and their licenses are listed in{" "}
          <code>THIRD_PARTY_NOTICES.md</code> in that repository.
        </p>
        <p>
          Installing a newer release keeps your settings and history. Uninstalling keeps them too
          unless you select <strong>Delete app data</strong> in the uninstaller.
        </p>
        {openError ? <p className="provider-consent-error">{openError}</p> : null}
      </div>
      <div className="about-actions">
        <button type="button" onClick={() => void open("open_data_folder")}>
          Open data folder
        </button>
        <button type="button" onClick={() => void open("open_latest_release")}>
          View latest release
        </button>
      </div>
    </section>
  );
}
