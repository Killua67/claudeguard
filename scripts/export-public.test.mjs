import { test } from "node:test";
import assert from "node:assert/strict";
import {
  mkdtempSync,
  writeFileSync,
  readFileSync,
  rmSync,
  existsSync,
  symlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { collectPublic, exportPublic } from "./export-public.mjs";
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "guard-export-"));
  const list = [
    "README.md",
    "LICENSE",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "public-files.json",
    "safe.rs",
  ];
  for (const file of list)
    writeFileSync(
      join(root, file),
      file === "public-files.json" ? JSON.stringify(list) : "public fixture\n",
    );
  return {
    root,
    list,
    cleanup: () => rmSync(root, { recursive: true, force: true }),
  };
}
test("exports only exact public files and leaves private history behind", () => {
  const f = fixture();
  const destination = join(f.root, "claudeguard");
  try {
    writeFileSync(join(f.root, "HANDOFF.md"), "private");
    assert.equal(exportPublic(f.root, destination), 6);
    assert.equal(existsSync(join(destination, "HANDOFF.md")), false);
    assert.match(
      readFileSync(join(destination, "PUBLIC-SHA256SUMS"), "utf8"),
      /safe.rs/,
    );
    assert.throws(() => exportPublic(f.root, destination));
  } finally {
    f.cleanup();
  }
});
test("rejects paths outside the manifest boundary, secrets and symbolic links before writing", () => {
  for (const kind of [
    "traversal",
    "drive-relative",
    "ads",
    "private",
    "secret",
    "symlink",
    "duplicate",
  ]) {
    const f = fixture();
    const destination = join(f.root, "claudeguard");
    try {
      if (kind === "traversal") f.list.push("../outside.rs");
      if (kind === "drive-relative") f.list.push("C:../outside.rs");
      if (kind === "ads") f.list.push("safe.rs:stream");
      if (kind === "private") f.list.push("HANDOFF.md");
      if (kind === "duplicate") f.list.push("safe.rs");
      if (kind === "secret")
        writeFileSync(
          join(f.root, "safe.rs"),
          ["", "Users", "private-person", "work", "secret"].join("/"),
        );
      if (kind === "symlink") {
        rmSync(join(f.root, "safe.rs"));
        symlinkSync("README.md", join(f.root, "safe.rs"));
      }
      writeFileSync(join(f.root, "public-files.json"), JSON.stringify(f.list));
      if (kind === "drive-relative" || kind === "ads") {
        assert.throws(() => collectPublic(f.root), /公开清单包含不允许的路径/);
      } else {
        assert.throws(() => collectPublic(f.root));
      }
      assert.throws(() => exportPublic(f.root, destination));
      assert.equal(existsSync(destination), false);
    } finally {
      f.cleanup();
    }
  }
});
