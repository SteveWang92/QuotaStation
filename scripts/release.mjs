// Three-phase release automation for QuotaStation.
//
// Phase 1 — prep:  bump version, finalize changelog, push dev, open/refresh release PR
//   node scripts/release.mjs prep [--version X.Y.Z] [--dry-run]
//
//   reversion:     change the version of a prepped release, files left uncommitted
//   node scripts/release.mjs reversion X.Y.Z
//
// Phase 2 — ship:  verify the PR can merge, then squash-merge, tag, GitHub release,
//                   reset dev. Pushes only the tag and the reset dev.
//   node scripts/release.mjs ship [--dry-run]
//
// Every phase is non-interactive — no TTY prompts. Safe for AI and CI use.

import { spawnSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  compareVersions,
  incrementVersion,
  parseVersion,
  suggestLevel,
} from "./release-version.mjs";

const projectRoot = process.cwd();
const gitSafeDirectory = projectRoot.replaceAll("\\", "/");
const args = process.argv.slice(2);
const subcommand = args.find((a) => !a.startsWith("--"));
const dryRun = args.includes("--dry-run");
const versionFlag = (() => {
  const idx = args.indexOf("--version");
  return idx >= 0 && args[idx + 1] ? args[idx + 1] : null;
})();

const APP_NAME = "QuotaStation";
const DEPLOY_BRANCH = "main";
const INTEGRATION_BRANCH = "dev";
const REPO = "SteveWang92/QuotaStation";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const run = (command, runArgs, options = {}) => {
  const result = spawnSync(command, runArgs, {
    cwd: projectRoot,
    encoding: "utf8",
    stdio: options.inherit ? "inherit" : "pipe",
    shell: options.shell ?? false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      options.inherit
        ? `${command} failed with exit code ${result.status}.`
        : result.stderr?.trim() || `${command} failed.`,
    );
  }
  return result.stdout?.trim() ?? "";
};

const git = (gitArgs) => run("git", ["-c", `safe.directory=${gitSafeDirectory}`, ...gitArgs]);

const gh = (ghArgs) => run("gh", ghArgs);

const latestTag = () => {
  const tags = git(["tag", "--list", "v[0-9]*", "--sort=-version:refname"])
    .split(/\r?\n/)
    .filter(Boolean);
  const tag = tags.find((t) => parseVersion(t.slice(1)));
  return tag ? { tag, version: tag.slice(1) } : null;
};

const commitsSince = (tag) => {
  const range = tag ? `${tag}..HEAD` : "HEAD";
  const output = git(["log", range, "--format=%s%x1f%b%x1e"]);
  return output
    .split("\x1e")
    .map((entry) => {
      const [subject = "", body = ""] = entry.trim().split("\x1f");
      return { subject: subject.trim(), body: body.trim() };
    })
    .filter((c) => c.subject);
};

const today = () => {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

const writeJson = (file, value) => writeFile(file, `${JSON.stringify(value, null, 2)}\n`, "utf8");

// ---------------------------------------------------------------------------
// Changelog helpers
// ---------------------------------------------------------------------------

const CHANGELOG_PATH = join(projectRoot, "CHANGELOG.md");

const readChangelog = () => readFile(CHANGELOG_PATH, "utf8");

const parseUnreleased = (content) => {
  const heading = "## [Unreleased]";
  const start = content.indexOf(heading);
  if (start < 0) throw new Error("No [Unreleased] section in CHANGELOG.md.");
  const afterHeading = content.indexOf("\n", start) + 1;
  const nextSection = content.indexOf("\n## [", afterHeading);
  const body =
    nextSection >= 0 ? content.slice(afterHeading, nextSection) : content.slice(afterHeading);
  return body.trim();
};

const finalizeChangelog = async (version, prevTag) => {
  let content = await readChangelog();
  const tag = `v${version}`;
  const dateStr = today();

  const heading = "## [Unreleased]";
  const headingPos = content.indexOf(heading);
  if (headingPos < 0) throw new Error("No [Unreleased] section in CHANGELOG.md.");
  const afterHeading = headingPos + heading.length;
  const nextSection = content.indexOf("\n## [", afterHeading);

  const unreleasedBody =
    nextSection >= 0 ? content.slice(afterHeading, nextSection) : content.slice(afterHeading);

  const before = content.slice(0, headingPos);
  const after = nextSection >= 0 ? content.slice(nextSection) : "";

  content = `${before}## [Unreleased]\n\n## [${version}] - ${dateStr}${unreleasedBody}${after}`;

  if (prevTag) {
    content = content.replace(
      `[Unreleased]: https://github.com/${REPO}/compare/${prevTag}...HEAD`,
      `[Unreleased]: https://github.com/${REPO}/compare/${tag}...HEAD\n[${version}]: https://github.com/${REPO}/compare/${prevTag}...${tag}`,
    );
  } else {
    const linksStart = content.lastIndexOf("\n[");
    const insertPos = linksStart >= 0 ? linksStart + 1 : content.length;
    const links = `[Unreleased]: https://github.com/${REPO}/compare/${tag}...HEAD\n[${version}]: https://github.com/${REPO}/releases/tag/${tag}\n`;
    content = content.slice(0, insertPos) + links + content.slice(insertPos);
  }

  await writeFile(CHANGELOG_PATH, content, "utf8");
};

// ---------------------------------------------------------------------------
// Version bump
// ---------------------------------------------------------------------------

// The renderer and the Rust core ship as one executable, so five files carry the version
// and all of them move together. The two src-tauri files and the Cargo lockfile are edited
// as text rather than re-serialised: tauri.conf.json is hand-formatted, and rewriting
// Cargo.lock through cargo would need a network-capable resolve for a version this script
// already knows.
const VERSION_FIELDS = [
  "package.json",
  "package-lock.json",
  "src-tauri/tauri.conf.json",
  "src-tauri/Cargo.toml",
  "src-tauri/Cargo.lock",
];

const CARGO_VERSION = /^version = "(\d+\.\d+\.\d+)"$/m;
const CARGO_LOCK_ENTRY = /(\[\[package\]\]\nname = "quotastation"\nversion = ")(\d+\.\d+\.\d+)(")/;
const TAURI_VERSION = /^( {2}"version": ")(\d+\.\d+\.\d+)(",)$/m;

const replaceOnce = async (relPath, pattern, replacer) => {
  const file = join(projectRoot, relPath);
  const content = await readFile(file, "utf8");
  const matches = content.match(
    new RegExp(pattern.source, pattern.flags.includes("g") ? pattern.flags : `${pattern.flags}g`),
  );
  if (matches?.length !== 1) {
    throw new Error(
      `Expected exactly one version field in ${relPath}, found ${matches?.length ?? 0}.`,
    );
  }
  await writeFile(file, content.replace(pattern, replacer), "utf8");
};

const readVersionFields = async () => {
  const pkg = JSON.parse(await readFile(join(projectRoot, "package.json"), "utf8"));
  const lock = JSON.parse(await readFile(join(projectRoot, "package-lock.json"), "utf8"));
  const tauri = await readFile(join(projectRoot, "src-tauri/tauri.conf.json"), "utf8");
  const cargo = await readFile(join(projectRoot, "src-tauri/Cargo.toml"), "utf8");
  const cargoLock = await readFile(join(projectRoot, "src-tauri/Cargo.lock"), "utf8");
  return [
    ["package.json", pkg.version],
    ["package-lock.json", lock.version],
    ['package-lock.json packages[""]', lock.packages?.[""]?.version],
    ["src-tauri/tauri.conf.json", tauri.match(TAURI_VERSION)?.[2]],
    ["src-tauri/Cargo.toml", cargo.match(CARGO_VERSION)?.[1]],
    ["src-tauri/Cargo.lock", cargoLock.match(CARGO_LOCK_ENTRY)?.[2]],
  ];
};

const bumpVersionFields = async (version) => {
  const pkgPath = join(projectRoot, "package.json");
  const pkg = JSON.parse(await readFile(pkgPath, "utf8"));
  pkg.version = version;
  await writeJson(pkgPath, pkg);

  const lockPath = join(projectRoot, "package-lock.json");
  const lock = JSON.parse(await readFile(lockPath, "utf8"));
  if ("version" in lock) lock.version = version;
  if (lock.packages?.[""]) lock.packages[""].version = version;
  await writeJson(lockPath, lock);

  await replaceOnce(
    "src-tauri/tauri.conf.json",
    TAURI_VERSION,
    (_m, before, _old, after) => `${before}${version}${after}`,
  );
  await replaceOnce("src-tauri/Cargo.toml", CARGO_VERSION, () => `version = "${version}"`);
  await replaceOnce(
    "src-tauri/Cargo.lock",
    CARGO_LOCK_ENTRY,
    (_m, before, _old, after) => `${before}${version}${after}`,
  );
};

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

// A release is "prepped" once the version bump and the dated changelog section are
// committed on dev but not yet tagged. Read from the files rather than from git history so
// it stays true after review fixes land on top; the tag check is what distinguishes a
// pending release from the one that shipped last time, since dev keeps both of those files
// at the released version afterwards.
const preppedVersion = async () => {
  const pkgVersion = JSON.parse(await readFile(join(projectRoot, "package.json"), "utf8")).version;
  const content = await readChangelog();
  if (!content.includes(`## [${pkgVersion}] - `)) return null;
  return git(["tag", "--list", `v${pkgVersion}`]) ? null : pkgVersion;
};

const releasedSection = (content, version) => {
  const heading = `## [${version}]`;
  const start = content.indexOf(heading);
  if (start < 0) return "";
  const afterLine = content.indexOf("\n", start) + 1;
  const nextHeading = content.indexOf("\n## [", afterLine);
  let body =
    nextHeading >= 0
      ? content.slice(afterLine, nextHeading).trim()
      : content.slice(afterLine).trim();
  const linksStart = body.indexOf("\n[Unreleased]:");
  if (linksStart >= 0) body = body.slice(0, linksStart).trim();
  return body;
};

const openReleasePR = () => {
  const list = JSON.parse(
    gh([
      "pr",
      "list",
      "--repo",
      REPO,
      "--head",
      INTEGRATION_BRANCH,
      "--base",
      DEPLOY_BRANCH,
      "--state",
      "open",
      "--json",
      "number,title",
      "--limit",
      "1",
    ]),
  );
  return list[0] ?? null;
};

// After the squash-merge the release PR is closed, so a failure in a later step
// would leave ship with nothing to find. Locating the merged PR lets a rerun
// resume the tag, the GitHub release, and the dev reset.
const mergedReleasePR = () => {
  const list = JSON.parse(
    gh([
      "pr",
      "list",
      "--repo",
      REPO,
      "--head",
      INTEGRATION_BRANCH,
      "--base",
      DEPLOY_BRANCH,
      "--state",
      "merged",
      "--json",
      "number,title",
      "--limit",
      "1",
    ]),
  );
  return list[0] ?? null;
};

const versionFromTitle = (title) => {
  const match = title.match(/v(\d+\.\d+\.\d+)/);
  if (!match) {
    throw new Error(`Could not read a version from the PR title "${title}".`);
  }
  return match[1];
};

const tagExists = (tag) => Boolean(git(["tag", "--list", tag]));

// Only "release not found" means the release is missing. Auth, network, and rate-limit
// failures surface instead, so they are never mistaken for an unfinished release.
const githubReleaseExists = (tag) => {
  try {
    gh(["release", "view", tag, "--repo", REPO, "--json", "tagName"]);
    return true;
  } catch (e) {
    if (/release not found/i.test(e.message)) return false;
    throw e;
  }
};

const isAncestor = (ancestor, descendant) => {
  try {
    git(["merge-base", "--is-ancestor", ancestor, descendant]);
    return true;
  } catch {
    return false;
  }
};

// True once origin's integration branch carries the squash-merged deploy branch — i.e. the
// reset reached origin or dev has moved on past it. Local dev alone is not enough: it is
// reset before the push, so a failed push leaves it carrying main while origin does not.
const integrationContainsDeploy = () =>
  isAncestor(`origin/${DEPLOY_BRANCH}`, `origin/${INTEGRATION_BRANCH}`);

const requireCleanIntegrationBranch = () => {
  const branch = git(["branch", "--show-current"]);
  if (branch !== INTEGRATION_BRANCH) {
    throw new Error(`Must be on ${INTEGRATION_BRANCH}, currently on ${branch || "detached HEAD"}.`);
  }
  if (git(["status", "--porcelain"])) {
    throw new Error("Working tree is not clean. Commit or stash changes first.");
  }
};

// ---------------------------------------------------------------------------
// prep — bump, finalize the changelog, push dev, open (or refresh) the release PR
//
// Idempotent: re-running after review fixes have landed re-pushes dev and refreshes the
// PR body without touching the already-committed version bump.
// ---------------------------------------------------------------------------

const prep = async () => {
  requireCleanIntegrationBranch();

  git(["fetch", "origin"]);
  git(["merge", "--ff-only", `origin/${INTEGRATION_BRANCH}`]);

  const latest = latestTag();
  const prepped = await preppedVersion();
  let version;

  if (prepped) {
    if (versionFlag && versionFlag !== prepped) {
      throw new Error(
        `v${prepped} is already prepped on ${INTEGRATION_BRANCH}. To release ${versionFlag} instead, run ` +
          `"node scripts/release.mjs reversion ${versionFlag}" and commit the result with your review fix.`,
      );
    }
    version = prepped;
  } else {
    const unreleased = parseUnreleased(await readChangelog());
    if (!unreleased) {
      throw new Error(
        "[Unreleased] section in CHANGELOG.md is empty. Add changelog entries before releasing.",
      );
    }
    const commits = commitsSince(latest?.tag);
    const level = suggestLevel(commits);
    const currentPkgVersion = JSON.parse(
      await readFile(join(projectRoot, "package.json"), "utf8"),
    ).version;
    const suggested = incrementVersion(latest?.version ?? currentPkgVersion, level);
    version = versionFlag ?? suggested;

    if (!parseVersion(version)) {
      throw new Error(`Invalid version "${version}". Use major.minor.patch.`);
    }
    if (latest && compareVersions(version, latest.version) <= 0) {
      throw new Error(`Version ${version} must be newer than ${latest.tag}.`);
    }
    if (git(["tag", "--list", `v${version}`])) {
      throw new Error(`Tag v${version} already exists.`);
    }

    console.log(`App:          ${APP_NAME}`);
    console.log(`Latest tag:   ${latest?.tag ?? "none"}`);
    console.log(`Commits:      ${commits.length}`);
    console.log(`Suggested:    ${level} -> ${suggested}`);
    console.log("");
    console.log("Unreleased changelog:");
    console.log(unreleased);
    console.log("");
  }

  console.log(`Version:      ${version}${prepped ? " (already bumped and committed)" : ""}`);
  console.log("");

  if (dryRun) {
    console.log("Dry run complete. Nothing committed, pushed, or opened.");
    return;
  }

  if (!prepped) {
    await bumpVersionFields(version);
    await finalizeChangelog(version, latest?.tag);
    git(["add", ...VERSION_FIELDS, "CHANGELOG.md"]);
    git(["commit", "-m", `chore(release): v${version}`]);
    console.log(`Committed chore(release): v${version}`);
  }

  git(["push", "origin", INTEGRATION_BRANCH]);
  console.log(`Pushed ${INTEGRATION_BRANCH}.`);

  const notes = releasedSection(await readChangelog(), version);
  const body = `## Changelog\n\n${notes}`;
  // The PR title becomes the squash subject verbatim, so it must be a Conventional Commit
  // line. GitHub appends " (#N)" to it — passing --subject at merge time would not, which
  // is why ship never overrides it.
  const title = `chore(release): v${version}`;

  const existing = openReleasePR();
  if (existing) {
    gh(["pr", "edit", String(existing.number), "--repo", REPO, "--title", title, "--body", body]);
    console.log(`PR #${existing.number} updated.`);
  } else {
    const prUrl = gh([
      "pr",
      "create",
      "--repo",
      REPO,
      "--base",
      DEPLOY_BRANCH,
      "--head",
      INTEGRATION_BRANCH,
      "--title",
      title,
      "--body",
      body,
    ]);
    console.log(`PR created: ${prUrl}`);
  }

  console.log("");
  console.log("Next steps:");
  console.log("  1. Review the PR; commit fixes on dev and re-run prep to refresh it");
  console.log(
    "  2. If the review changes the version level, run: node scripts/release.mjs reversion X.Y.Z",
  );
  console.log("  3. Run: node scripts/release.mjs ship");
};

// ---------------------------------------------------------------------------
// reversion — change the version of an already-prepped release
//
// Rewrites the four places the version lives (version fields, changelog heading, compare
// links, PR title) and leaves the file changes uncommitted so they can go in with the
// review fix that motivated the change.
// ---------------------------------------------------------------------------

const reversion = async () => {
  const positional = args.filter((a) => !a.startsWith("--"))[1] ?? null;
  const target = versionFlag ?? positional;
  if (!target) {
    throw new Error("Usage: node scripts/release.mjs reversion X.Y.Z");
  }
  if (!parseVersion(target)) {
    throw new Error(`Invalid version "${target}". Use major.minor.patch.`);
  }

  requireCleanIntegrationBranch();
  git(["fetch", "origin"]);
  git(["merge", "--ff-only", `origin/${INTEGRATION_BRANCH}`]);

  const current = await preppedVersion();
  if (!current) {
    throw new Error(`No prepped release found on ${INTEGRATION_BRANCH}. Run "prep" first.`);
  }
  if (current === target) {
    throw new Error(`The prepped release is already v${target}.`);
  }

  const latest = latestTag();
  if (git(["tag", "--list", `v${target}`])) {
    throw new Error(`Tag v${target} already exists.`);
  }
  if (latest && compareVersions(target, latest.version) <= 0) {
    throw new Error(`Version ${target} must be newer than ${latest.tag}.`);
  }

  console.log(`Re-versioning: v${current} -> v${target}`);

  if (dryRun) {
    console.log("Dry run complete. No files or PR touched.");
    return;
  }

  await bumpVersionFields(target);

  const content = await readChangelog();
  const rewritten = content
    .replace(`## [${current}] - `, `## [${target}] - `)
    .replace(`\n[${current}]: `, `\n[${target}]: `)
    .replaceAll(`...v${current}`, `...v${target}`)
    .replace(`compare/v${current}...HEAD`, `compare/v${target}...HEAD`)
    .replaceAll(`/tag/v${current}`, `/tag/v${target}`);
  await writeFile(CHANGELOG_PATH, rewritten, "utf8");
  console.log("Rewrote the version files and CHANGELOG.md.");

  const existing = openReleasePR();
  if (existing) {
    gh([
      "pr",
      "edit",
      String(existing.number),
      "--repo",
      REPO,
      "--title",
      `chore(release): v${target}`,
    ]);
    console.log(`PR #${existing.number} title updated.`);
  } else {
    console.log("No open release PR to retitle.");
  }

  console.log("");
  console.log(
    "File changes are left uncommitted — commit them with the review fix that caused the version change.",
  );
};

// ---------------------------------------------------------------------------
// ship — verify, merge, tag, publish, reset
//
// Everything that could block the merge is checked first, so a failed pre-flight leaves the
// release exactly as it was. Fix the cause (or wait for CI) and run ship again. Past the
// merge the PR is closed, and ship resumes from the merged PR — rerunning is always safe.
// ---------------------------------------------------------------------------

const describeChecks = (rollup) => {
  const bad = (rollup ?? [])
    .map((c) => ({
      name: c.name ?? c.context ?? "check",
      state: c.conclusion || c.state || c.status || "PENDING",
    }))
    .filter((c) => !["SUCCESS", "NEUTRAL", "SKIPPED"].includes(String(c.state).toUpperCase()));
  return bad.length
    ? ` Checks not passing: ${bad.map((c) => `${c.name} (${c.state})`).join(", ")}.`
    : "";
};

// The version the PR title names must be the one every version field and the dated
// changelog section carry. Shared by the open-PR pre-flight and the merged-PR resume.
const versionFieldProblems = async (version) => {
  const problems = [];

  for (const [field, value] of await readVersionFields()) {
    if (value !== version) {
      problems.push(
        `PR title says v${version} but ${field} is ${value ?? "missing"}. Use "reversion" to change the version everywhere at once.`,
      );
    }
  }

  const changelog = await readChangelog();
  if (!changelog.includes(`## [${version}] - `)) {
    problems.push(`CHANGELOG.md has no "## [${version}] - <date>" section. Run "prep" first.`);
  }

  return problems;
};

const preflight = async (pr, version) => {
  const problems = [];
  const expectedTitle = `chore(release): v${version}`;

  if (pr.title !== expectedTitle) {
    problems.push(`PR title must be exactly "${expectedTitle}", found "${pr.title}".`);
  }

  const localHead = git(["rev-parse", "HEAD"]);
  if (pr.headRefOid !== localHead) {
    problems.push(
      `PR head ${pr.headRefOid.slice(0, 7)} does not match local ${INTEGRATION_BRANCH} head ${localHead.slice(0, 7)}. Run "prep" to push.`,
    );
  }

  problems.push(...(await versionFieldProblems(version)));

  if (git(["tag", "--list", `v${version}`])) {
    problems.push(`Tag v${version} already exists.`);
  }
  const latest = latestTag();
  if (latest && compareVersions(version, latest.version) <= 0) {
    problems.push(`Version ${version} is not newer than ${latest.tag}.`);
  }

  if (pr.mergeStateStatus !== "CLEAN") {
    problems.push(
      `GitHub reports the PR as ${pr.mergeStateStatus} (mergeable: ${pr.mergeable}).${describeChecks(pr.statusCheckRollup)}`,
    );
  }

  return problems;
};

const ship = async () => {
  if (versionFlag) {
    throw new Error(
      'ship takes no --version. The version comes from the release PR title; use "reversion" to change it.',
    );
  }

  requireCleanIntegrationBranch();

  git(["fetch", "origin"]);
  // Step 5 resets dev to main before force-pushing it. When that push is the step that
  // failed, local dev already is main while origin/dev still holds the pre-merge branch, so
  // there is nothing to fast-forward — only the push to retry. The squash put dev's tree on
  // main, so matching trees tell that outstanding push apart from an origin/dev carrying
  // work the retried force-push would discard.
  const devPushPending =
    git(["rev-parse", "HEAD"]) === git(["rev-parse", `origin/${DEPLOY_BRANCH}`]) &&
    !integrationContainsDeploy() &&
    !git(["diff", "--stat", `origin/${DEPLOY_BRANCH}`, `origin/${INTEGRATION_BRANCH}`]);
  if (!devPushPending) {
    git(["merge", "--ff-only", `origin/${INTEGRATION_BRANCH}`]);
  }

  const open = openReleasePR();
  const releasePR = open ?? mergedReleasePR();
  if (!releasePR) {
    throw new Error('No release PR found. Run "prep" first.');
  }

  const version = versionFromTitle(releasePR.title);
  const releaseTag = `v${version}`;

  if (open) {
    const pr = JSON.parse(
      gh([
        "pr",
        "view",
        String(open.number),
        "--repo",
        REPO,
        "--json",
        "number,title,mergeable,mergeStateStatus,headRefOid,statusCheckRollup",
      ]),
    );
    console.log(`Found PR: #${pr.number} "${pr.title}"`);

    const problems = await preflight(pr, version);
    if (problems.length > 0) {
      console.error(`Cannot ship ${releaseTag} yet:`);
      for (const problem of problems) console.error(`  - ${problem}`);
      console.error("");
      console.error(
        "Nothing has been changed. Fix the cause — or wait a few minutes if checks are still running — and run ship again.",
      );
      process.exit(1);
    }

    console.log(`Version:    ${version}`);
    console.log(`Tag:        ${releaseTag}`);
    console.log("Pre-flight: all clear");
    console.log("");

    if (dryRun) {
      console.log("Dry run complete. No changes made.");
      return;
    }

    // 1. Squash merge the PR — no --subject, so GitHub uses the PR title and appends
    //    " (#N)". Only the auto-generated body is stripped.
    console.log(`Squash-merging PR #${pr.number}...`);
    gh(["pr", "merge", String(pr.number), "--repo", REPO, "--squash", "--body", ""]);
    console.log("PR merged.");
  } else {
    // The merge closes the PR, so a failure in any later step lands here on the
    // next run. Carry on with whichever steps are still outstanding.
    if (tagExists(releaseTag) && githubReleaseExists(releaseTag) && integrationContainsDeploy()) {
      throw new Error(
        `No open release PR, and ${releaseTag} is already complete. Run "prep" to start the next release.`,
      );
    }

    const problems = await versionFieldProblems(version);
    if (problems.length > 0) {
      console.error(
        `Cannot resume ${releaseTag}; the merged PR title does not match the release files:`,
      );
      for (const problem of problems) console.error(`  - ${problem}`);
      process.exit(1);
    }

    console.log(
      `PR #${releasePR.number} for ${releaseTag} is already merged; resuming the remaining steps.`,
    );
    console.log("");

    if (dryRun) {
      console.log("Dry run complete. No changes made.");
      return;
    }
  }

  // 2. Sync main from remote
  git(["checkout", DEPLOY_BRANCH]);
  git(["pull", "origin", DEPLOY_BRANCH]);
  console.log(`Synced ${DEPLOY_BRANCH}.`);

  // The squash must have carried dev's tree onto main. Anything else means the
  // merge did not produce the reviewed content, so stop before tagging it.
  const contentDiff = git(["diff", "--stat", DEPLOY_BRANCH, INTEGRATION_BRANCH]);
  if (contentDiff) {
    throw new Error(
      `${DEPLOY_BRANCH} and ${INTEGRATION_BRANCH} have different content after the release PR merged:\n${contentDiff}\n` +
        `Resolve the difference before tagging or resetting ${INTEGRATION_BRANCH}.`,
    );
  }
  console.log(`${DEPLOY_BRANCH} and ${INTEGRATION_BRANCH} content trees match.`);

  // 3. Create annotated tag and push
  if (tagExists(releaseTag)) {
    console.log(`Tag ${releaseTag} already exists.`);
  } else {
    git(["tag", "-a", releaseTag, "-m", `${APP_NAME} ${version}`]);
  }
  git(["push", "origin", releaseTag]);
  console.log(`Tag ${releaseTag} is on origin.`);

  // 4. Create GitHub release with the changelog section as notes
  if (githubReleaseExists(releaseTag)) {
    console.log(`GitHub release ${releaseTag} already exists.`);
  } else {
    const releaseNotes = releasedSection(await readChangelog(), version);
    gh([
      "release",
      "create",
      releaseTag,
      "--repo",
      REPO,
      "--title",
      releaseTag,
      "--notes",
      releaseNotes || `${APP_NAME} ${version}`,
    ]);
    console.log(`GitHub release ${releaseTag} created.`);
  }

  // 5. Reset dev to main and force push
  git(["checkout", INTEGRATION_BRANCH]);
  if (integrationContainsDeploy()) {
    console.log(`${INTEGRATION_BRANCH} already carries ${DEPLOY_BRANCH}.`);
  } else {
    git(["reset", "--hard", DEPLOY_BRANCH]);
    git(["push", "--force-with-lease", "origin", INTEGRATION_BRANCH]);
    console.log(`Reset ${INTEGRATION_BRANCH} to ${DEPLOY_BRANCH} and force-pushed.`);
  }

  console.log("");
  console.log(`Release ${version} complete.`);
  console.log("");
  console.log("Publishing the release starts .github/workflows/release.yml, which builds the NSIS");
  console.log(
    "installer from the tag and attaches it. Confirm that run finished and the installer is",
  );
  console.log(`on the release before calling ${releaseTag} done:`);
  console.log(`  gh run list --repo ${REPO} --workflow release.yml --limit 1`);
};

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

if (subcommand === "prep") {
  await prep();
} else if (subcommand === "reversion") {
  await reversion();
} else if (subcommand === "ship") {
  await ship();
} else {
  console.error(
    "Usage: node scripts/release.mjs <prep|reversion|ship> [--version X.Y.Z] [--dry-run]",
  );
  console.error("");
  console.error(
    "  prep            Bump, finalize the changelog, push dev, open or refresh the release PR",
  );
  console.error(
    "  reversion X.Y.Z Change the version of a prepped release (files left uncommitted)",
  );
  console.error("  ship            Verify the PR can merge, then merge, tag, and publish");
  process.exit(1);
}
