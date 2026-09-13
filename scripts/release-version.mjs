// Pure semver helpers shared by release scripts.
// No repo-specific knowledge lives here.

export const parseVersion = (value) => {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(value);
  return match ? match.slice(1).map(Number) : null;
};

export const compareVersions = (left, right) => {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) throw new Error("Version comparison requires valid semver.");
  for (let index = 0; index < a.length; index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index];
  }
  return 0;
};

export const incrementVersion = (version, level) => {
  const parsed = parseVersion(version);
  if (!parsed) throw new Error(`Invalid current version: ${version}`);
  const [major, minor, patch] = parsed;
  if (level === "major") return `${major + 1}.0.0`;
  if (level === "feature") return `${major}.${minor + 1}.0`;
  return `${major}.${minor}.${patch + 1}`;
};

export const suggestLevel = (commits) => {
  const breaking = commits.some(
    ({ subject, body }) =>
      /^[a-z]+(?:\([^)]*\))?!:/i.test(subject) || /BREAKING[ -]CHANGE:/i.test(body),
  );
  if (breaking) return "major";
  if (commits.some(({ subject }) => /^feat(?:\([^)]*\))?:/i.test(subject))) {
    return "feature";
  }
  return "fix";
};
