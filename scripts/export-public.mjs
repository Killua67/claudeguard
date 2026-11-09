#!/usr/bin/env node
// Exact-file export; never walks the private workspace or executes source files.
import {
  lstatSync,
  readFileSync,
  mkdirSync,
  writeFileSync,
  rmSync,
} from "node:fs";
import { resolve, dirname, relative, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

export function collectPublic(root) {
  root = resolve(root);
  const list = JSON.parse(
    readFileSync(resolve(root, "public-files.json"), "utf8"),
  );
  if (
    !Array.isArray(list) ||
    list.length === 0 ||
    new Set(list).size !== list.length
  )
    throw Error("公开清单无效或重复");
  for (const name of [
    "README.md",
    "LICENSE",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "public-files.json",
  ]) {
    if (!list.includes(name)) throw Error(`缺少必要公开文件：${name}`);
  }
  const forbidden =
    /(^|\/)(?:HANDOFF\.md|Design|Experiments|Sources|Tests|node_modules|target|dist|output|\.tooling|\.build|\.git|\.env(?:\..*)?)(\/|$)/;
  const textExtensions =
    /\.(?:rs|toml|lock|json|md|html|ts|tsx|css|sh|mjs|yml|yaml)$/;
  const binary =
    /^apps\/desktop\/src-tauri\/icons\/(?:32x32\.png|128x128\.png|128x128@2x\.png|icon\.icns|icon\.ico)$/;
  return list
    .slice()
    .sort()
    .map((name) => {
      if (
        typeof name !== "string" ||
        isAbsolute(name) ||
        name.includes(":") ||
        name.includes("\\") ||
        name.split("/").some((p) => !p || p === "." || p === "..") ||
        forbidden.test(name)
      )
        throw Error("公开清单包含不允许的路径");
      if (
        !textExtensions.test(name) &&
        !binary.test(name) &&
        !["LICENSE", "NOTICE", ".gitignore", ".gitattributes"].includes(name)
      )
        throw Error(`未审核的文件类型：${name}`);
      const resolved = resolve(root, name);
      const within = relative(root, resolved);
      if (
        within === ".." ||
        within.startsWith("../") ||
        within.startsWith("..\\") ||
        isAbsolute(within)
      )
        throw Error("路径超出公开根目录");
      let path = root;
      for (const part of name.split("/")) {
        path = resolve(path, part);
        const stat = lstatSync(path);
        if (stat.isSymbolicLink()) throw Error(`禁止软链接：${name}`);
      }
      const stat = lstatSync(path);
      if (!stat.isFile() || stat.size > 4 * 1024 * 1024)
        throw Error(`文件类型或大小异常：${name}`);
      const bytes = readFileSync(path);
      if (!binary.test(name)) {
        const text = bytes.toString("utf8");
        const sensitive = [
          /\/Users\/[A-Za-z0-9_.-]+\//,
          /C:\\Users\\[A-Za-z0-9_.-]+\\/i,
          /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/,
          /\b(?:sk-ant-|github_pat_|ghp_)[A-Za-z0-9_-]{16,}/,
          /\bAKIA[A-Z0-9]{16}\b/,
        ];
        if (sensitive.some((pattern) => pattern.test(text)))
          throw Error(`发现疑似个人路径或密钥，请人工审核：${name}`);
      }
      return {
        name,
        bytes,
        hash: createHash("sha256").update(bytes).digest("hex"),
      };
    });
}
export function exportPublic(root, destination) {
  const files = collectPublic(root);
  const target = resolve(destination);
  if (target === resolve(root)) throw Error("不能覆盖开发目录");
  mkdirSync(target); // Exclusive: never overwrite any existing directory.
  try {
    for (const file of files) {
      const path = resolve(target, file.name);
      const within = relative(target, path);
      if (
        within === ".." ||
        within.startsWith("../") ||
        within.startsWith("..\\") ||
        isAbsolute(within)
      )
        throw Error("路径超出导出目录");
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, file.bytes, {
        flag: "wx",
        mode: file.name.endsWith(".sh") ? 0o755 : 0o644,
      });
    }
    writeFileSync(
      resolve(target, "PUBLIC-SHA256SUMS"),
      files.map((f) => `${f.hash}  ${f.name}`).join("\n") + "\n",
      { flag: "wx" },
    );
  } catch (error) {
    rmSync(target, { recursive: true, force: true });
    throw error;
  }
  return files.length;
}
if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
    const args = process.argv.slice(2);
    if (args.length === 1 && args[0] === "--check")
      console.log(
        `公开清单检查通过：${collectPublic(root).length} 个文件；仍需维护者审核。`,
      );
    else if (args.length === 2 && args[0] === "--destination")
      console.log(
        `已导出 ${exportPublic(root, args[1])} 个文件；未初始化 Git 或上传。`,
      );
    else
      throw Error(
        "Usage: node scripts/export-public.mjs --check | --destination /new/claudeguard",
      );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
