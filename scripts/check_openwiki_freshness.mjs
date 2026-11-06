#!/usr/bin/env node
/**
 * Audit OpenWiki Grounded Claims against current repository source.
 *
 * OpenWiki owns the line-range relocation/hash algorithm. Use the pinned resolver
 * rather than a second implementation: unchanged text that moved keeps its
 * version, while changed/missing/unsafe evidence is reported as stale. This is a
 * freshness audit only; it never changes or blesses a claim.
 */
import { lstat, readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const root = path.resolve(process.argv[2] ?? process.cwd());
const moduleRoot = process.env.OPENWIKI_MODULE_ROOT;
if (!moduleRoot) {
  console.error("OPENWIKI_MODULE_ROOT must point at the pinned OpenWiki package root");
  process.exit(2);
}
const { RepositoryEvidenceResolver } = await import(
  pathToFileURL(path.join(moduleRoot, "dist/claims/evidence/repository/resolver.js")).href
);
const resolver = new RepositoryEvidenceResolver({ rootDir: root });
const claimsRoot = path.join(root, "openwiki/.claims");
const pagesRoot = path.join(root, "openwiki");
const pageManifest = JSON.parse(await readFile(path.join(pagesRoot, ".page-manifest.json"), "utf8"));
if (pageManifest.schemaVersion !== 1 || typeof pageManifest.pages !== "object") {
  throw new Error("OpenWiki page manifest has an unsupported schema");
}

async function jsonFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const result = [];
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) result.push(...await jsonFiles(filename));
    else if (entry.isFile() && entry.name.endsWith(".json")) result.push(filename);
  }
  return result;
}

const stale = [];
let claimsChecked = 0;
let evidenceChecked = 0;
let pagesChecked = 0;
for (const filename of await jsonFiles(claimsRoot)) {
  const sidecar = JSON.parse(await readFile(filename, "utf8"));
  if (!Array.isArray(sidecar.claims) || typeof sidecar.pageVersion !== "string") {
    throw new Error(`${path.relative(root, filename)} has an invalid Claims sidecar`);
  }
  const page = path.relative(claimsRoot, filename).replace(/\.json$/u, ".md");
  const pagePath = path.resolve(pagesRoot, page);
  const contained = path.relative(pagesRoot, pagePath);
  if (contained.startsWith(`..${path.sep}`) || path.isAbsolute(contained)) {
    throw new Error(`Claims sidecar escapes openwiki/: ${page}`);
  }
  const pageStat = await lstat(pagePath).catch(() => null);
  if (!pageStat?.isFile() || pageStat.isSymbolicLink()) {
    stale.push({ page, reason: "Markdown page is missing or not a regular file" });
    continue;
  }
  await readFile(pagePath, "utf8");
  const manifestPath = `/openwiki/${page}`;
  if (pageManifest.pages[manifestPath]?.pageVersion !== sidecar.pageVersion) {
    stale.push({ page, reason: "page manifest and Claims sidecar versions differ" });
  }
  pagesChecked += 1;
  for (const claim of sidecar.claims) {
    claimsChecked += 1;
    if (!Array.isArray(claim.evidence) || claim.evidence.length === 0) {
      stale.push({ page, claim: claim.id ?? "unknown", reason: "no source evidence" });
      continue;
    }
    for (const item of claim.evidence) {
      evidenceChecked += 1;
      try {
        const current = await resolver.resolve(item.resource, item.version);
        if (!current) {
          stale.push({ page, claim: claim.id ?? "unknown", resource: item.resource, reason: "source or range disappeared" });
        } else if (current.evidence.version !== item.version) {
          stale.push({ page, claim: claim.id ?? "unknown", resource: item.resource, reason: "source evidence changed" });
        } else if (current.evidence.resource !== item.resource) {
          // The source text is unchanged but its line range moved. Report this
          // too, so citations can be refreshed without claiming the fact changed.
          stale.push({ page, claim: claim.id ?? "unknown", resource: item.resource, reason: `unchanged evidence moved to ${current.evidence.resource}` });
        }
      } catch (error) {
        stale.push({ page, claim: claim.id ?? "unknown", resource: item.resource, reason: String(error.message ?? error) });
      }
    }
  }
}

const report = {
  schema: 1,
  openwiki_version: "0.9.0",
  source_head: process.env.GITHUB_SHA ?? "local",
  pages_checked: pagesChecked,
  claims_checked: claimsChecked,
  evidence_checked: evidenceChecked,
  stale_evidence: stale,
};
console.log(JSON.stringify(report, null, 2));
if (stale.length > 0) {
  const pages = [...new Set(stale.map((item) => item.page))].sort();
  console.error(`OpenWiki freshness: ${stale.length} finding(s) across ${pages.length} page(s): ${pages.join(", ")}`);
  process.exit(1);
}
console.log(`OpenWiki freshness: ${pagesChecked} pages, ${claimsChecked} claims and ${evidenceChecked} evidence references current.`);
