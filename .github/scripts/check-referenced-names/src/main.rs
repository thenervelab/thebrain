//! Forward-looking §13 name-existence check, for **unreleased** runtime PRs.
//!
//! The indexer (`hippius-indexer`, `indexer-rs/xtask/src/check_names.rs`) checks
//! every chain name it references against the *pinned, compiled* runtime metadata
//! (`metadata/*.scale`) — the authoritative record of what's actually deployed. That
//! check can't see a PR *in this repo* that renames or removes a storage item, event,
//! or call before it merges and gets compiled into a new metadata artifact, because
//! no `.scale` exists yet for unmerged code.
//!
//! This is that missing half: it parses **this repo's own pallet source** (via
//! `syn`, not compiled metadata) to answer the same question — "does the indexer's
//! referenced surface still resolve?" — one PR earlier, before a name changes
//! silently reaches `dev`/`main` and only shows up as a live defect the *next* time
//! someone runs the indexer's own check against a fresh `.scale`.
//!
//! **WARN-only, deliberately.** This is the first CI check this repo has ever had.
//! Landing it as a required, blocking check on day one is how a check gets deleted
//! the first time it's inconvenient rather than fixed. It always exits `0`; findings
//! are printed as GitHub Actions `::warning::` annotations so they show up on the PR
//! without blocking merge.
//!
//! **Scope: this repo's own custom pallets only** (`pallets/*`), not upstream
//! Substrate/FRAME pallets (`System`, `Balances`, `Timestamp`, ...) — those live in
//! external crates this repo doesn't version, so there's no local source to parse for
//! them; the indexer's own metadata-based check already covers them at release time.
//! A referenced name whose pallet isn't a local pallet is reported as skipped, not
//! silently ignored and not treated as a failure.
//!
//! **The referenced-names surface is a bundled, manually-synced snapshot**
//! (`referenced_names.json`, copied from `hippius-indexer/indexer-rs/metadata/`), not
//! a live cross-repo fetch — simpler and more reliable in CI than depending on
//! network access to another repo, at the cost of needing an occasional manual
//! re-sync. Re-sync: copy `indexer-rs/metadata/referenced_names.json` over the copy
//! in this directory.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use syn::{Attribute, Item, ItemEnum, ItemImpl, ItemMod, ItemType, Visibility};

// ---------------------------------------------------------------------------
// referenced_names.json — the indexer's own referenced surface (bundled copy)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Referenced {
	archive_filters: Vec<ArchiveFilter>,
	storage_reads: Vec<StorageRead>,
}

#[derive(Deserialize)]
struct ArchiveFilter {
	pallet: String,
	item: String,
	#[serde(default)]
	readers: Vec<String>,
}

#[derive(Deserialize)]
struct StorageRead {
	pallet: String,
	item: String,
	#[serde(default)]
	readers: Vec<String>,
}

/// polkadot-js lower-cases the first character only — same convention the indexer's
/// own `check_names.rs::lower_first` uses; must match exactly, or a storage name
/// would resolve here and not there (or vice versa).
fn lower_first(s: &str) -> String {
	let mut c = s.chars();
	match c.next() {
		Some(first) => first.to_lowercase().collect::<String>() + c.as_str(),
		None => String::new(),
	}
}

fn readers_str(readers: &[String]) -> String {
	if readers.is_empty() {
		"none recorded".to_string()
	} else {
		readers.join(", ")
	}
}

// ---------------------------------------------------------------------------
// construct_runtime! — Runtime pallet name -> crate identifier
// ---------------------------------------------------------------------------

/// `construct_runtime!` is an opaque macro invocation as far as `syn` is concerned
/// (its body is a `TokenStream`, not structured items), so this is a targeted scan
/// over the raw source rather than an AST walk — robust to the one shape that
/// matters here: `PalletName: crate_ident(::<InstanceN>)? = N,`, one per line,
/// commented-out lines (`// Foo: ...`) skipped.
fn parse_construct_runtime(runtime_lib_rs: &str) -> Vec<(String, String)> {
	// Search for the real invocation (`construct_runtime!(`), not a mention of the
	// macro name in a comment (line 17 of this same file has exactly that).
	let Some(start) = runtime_lib_rs.find("construct_runtime!(") else {
		return Vec::new();
	};
	let body_start = start + "construct_runtime!".len();

	// Find the matching close paren, counting both `(`/`)` and `{`/`}` together
	// (the enum body inside uses braces) — depth returns to 0 at the real end.
	let bytes = runtime_lib_rs.as_bytes();
	let mut depth = 0i32;
	let mut end = body_start;
	for (i, &b) in bytes[body_start..].iter().enumerate() {
		match b {
			b'(' | b'{' => depth += 1,
			b')' | b'}' => {
				depth -= 1;
				if depth == 0 {
					end = body_start + i;
					break;
				}
			},
			_ => {},
		}
	}
	let body = &runtime_lib_rs[body_start..=end];

	let mut out = Vec::new();
	for raw_line in body.lines() {
		let line = raw_line.trim();
		if line.starts_with("//") || line.is_empty() {
			continue;
		}
		// `PalletName: crate_ident` up to `::<Instance..>` or `=`.
		let Some(colon) = line.find(':') else {
			continue;
		};
		let pallet_name = line[..colon].trim();
		if pallet_name.is_empty() || !pallet_name.chars().next().unwrap().is_uppercase() {
			continue;
		}
		let rest = &line[colon + 1..];
		let Some(eq) = rest.find('=') else { continue };
		let crate_part = rest[..eq].trim();
		// Strip an `::<Instance..>` suffix, keep the base crate identifier.
		let crate_ident = crate_part.split("::").next().unwrap_or("").trim();
		if crate_ident.is_empty()
			|| !crate_ident.starts_with("pallet") && crate_ident != "frame_system"
		{
			continue;
		}
		out.push((pallet_name.to_string(), crate_ident.to_string()));
	}
	out
}

// ---------------------------------------------------------------------------
// pallets/*/Cargo.toml — crate identifier -> local source directory
// ---------------------------------------------------------------------------

fn crate_ident_from_cargo_toml(cargo_toml: &Path) -> Option<String> {
	let text = fs::read_to_string(cargo_toml).ok()?;
	for line in text.lines() {
		let line = line.trim();
		if let Some(rest) = line.strip_prefix("name") {
			let rest = rest.trim_start();
			if let Some(rest) = rest.strip_prefix('=') {
				let name = rest.trim().trim_matches('"');
				return Some(name.replace('-', "_"));
			}
		}
		// Stop at the end of the `[package]` table's obvious first lines; cheap
		// enough not to bother detecting `[dependencies]` precisely.
		if line.starts_with('[') && line != "[package]" {
			break;
		}
	}
	None
}

fn discover_local_pallets(pallets_dir: &Path) -> BTreeMap<String, PathBuf> {
	let mut map = BTreeMap::new();
	let Ok(entries) = fs::read_dir(pallets_dir) else {
		return map;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if !path.is_dir() {
			continue;
		}
		let cargo_toml = path.join("Cargo.toml");
		if let Some(ident) = crate_ident_from_cargo_toml(&cargo_toml) {
			map.insert(ident, path.join("src"));
		}
	}
	map
}

// ---------------------------------------------------------------------------
// pallet source -> events / calls / storage (via syn)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PalletSurface {
	events: BTreeSet<String>,
	calls: BTreeSet<String>,
	/// JS form (lowerFirst) — matches the indexer's own storage-read convention.
	storage_js: BTreeSet<String>,
}

fn has_attr(attrs: &[Attribute], suffix: &str) -> bool {
	attrs
		.iter()
		.any(|a| a.path().segments.last().is_some_and(|s| s.ident == suffix))
}

fn walk_items(items: &[Item], surface: &mut PalletSurface) {
	for item in items {
		match item {
			Item::Enum(ItemEnum { ident, attrs, variants, .. })
				if ident == "Event" && has_attr(attrs, "event") =>
			{
				for v in variants {
					surface.events.insert(v.ident.to_string());
				}
			},
			Item::Type(ItemType { ident, attrs, .. }) if has_attr(attrs, "storage") => {
				surface.storage_js.insert(lower_first(&ident.to_string()));
			},
			Item::Impl(ItemImpl { attrs, items, .. }) if has_attr(attrs, "call") => {
				for impl_item in items {
					if let syn::ImplItem::Fn(f) = impl_item {
						if matches!(f.vis, Visibility::Public(_)) {
							surface.calls.insert(f.sig.ident.to_string());
						}
					}
				}
			},
			Item::Mod(ItemMod { content: Some((_, inner_items)), .. }) => {
				walk_items(inner_items, surface)
			},
			_ => {},
		}
	}
}

fn scan_pallet_source(src_dir: &Path) -> Result<PalletSurface> {
	let mut surface = PalletSurface::default();
	let mut stack = vec![src_dir.to_path_buf()];
	while let Some(dir) = stack.pop() {
		let Ok(entries) = fs::read_dir(&dir) else {
			continue;
		};
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				stack.push(path);
			} else if path.extension().is_some_and(|e| e == "rs") {
				let text = fs::read_to_string(&path)
					.with_context(|| format!("reading {}", path.display()))?;
				match syn::parse_file(&text) {
					Ok(file) => walk_items(&file.items, &mut surface),
					Err(e) => {
						// A parse failure shouldn't nuke the whole check — warn and
						// move on; this file's surface is simply unchecked this run.
						eprintln!(
							"::warning::check-referenced-names: could not parse {} ({e}) — skipped",
							path.display()
						);
					},
				}
			}
		}
	}
	Ok(surface)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
	// This crate lives at <repo>/.github/scripts/check-referenced-names/.
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("../../..")
		.canonicalize()
		.expect("repo root must exist")
}

fn main() -> Result<()> {
	let root = repo_root();

	let runtime_lib_rs = fs::read_to_string(root.join("runtime/mainnet/src/lib.rs"))
		.context("reading runtime/mainnet/src/lib.rs")?;
	let runtime_pallets = parse_construct_runtime(&runtime_lib_rs);
	if runtime_pallets.is_empty() {
		anyhow::bail!("parsed zero pallets out of construct_runtime! — the parser is broken");
	}

	let local_pallets = discover_local_pallets(&root.join("pallets"));

	let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let referenced: Referenced = serde_json::from_str(
		&fs::read_to_string(manifest_dir.join("referenced_names.json"))
			.context("reading bundled referenced_names.json")?,
	)
	.context("parsing bundled referenced_names.json")?;

	// Build a surface only for the local pallets construct_runtime! actually names,
	// and only the ones the indexer's referenced_names.json even mentions — no need
	// to parse pallets nothing references.
	let referenced_pallet_names: BTreeSet<String> = referenced
		.archive_filters
		.iter()
		.map(|f| f.pallet.clone())
		.chain(referenced.storage_reads.iter().map(|s| lower_first(&s.pallet)))
		.collect();

	let mut surfaces: BTreeMap<String, PalletSurface> = BTreeMap::new();
	let mut unresolved_pallets: BTreeSet<String> = BTreeSet::new();
	for (pallet_name, crate_ident) in &runtime_pallets {
		if !referenced_pallet_names.contains(pallet_name)
			&& !referenced_pallet_names.contains(&lower_first(pallet_name))
		{
			continue;
		}
		match local_pallets.get(crate_ident) {
			Some(src_dir) => {
				surfaces.insert(pallet_name.clone(), scan_pallet_source(src_dir)?);
			},
			None => {
				unresolved_pallets.insert(pallet_name.clone());
			},
		}
	}

	let mut dead: Vec<String> = Vec::new();
	let mut skipped: Vec<String> = Vec::new();

	let runtime_pallet_names: BTreeSet<&str> =
		runtime_pallets.iter().map(|(n, _)| n.as_str()).collect();

	for f in &referenced.archive_filters {
		if !runtime_pallet_names.contains(f.pallet.as_str()) {
			// Same case indexer's own check_names.rs calls "no pallet `X` in the
			// runtime" — genuinely dead, not out of scope, even though there's no
			// local source to blame it on.
			dead.push(format!(
				"archive filter {}.{} — no pallet `{}` in `construct_runtime!` at all \
                 (readers: {})",
				f.pallet,
				f.item,
				f.pallet,
				readers_str(&f.readers)
			));
			continue;
		}
		let Some(surface) = surfaces.get(&f.pallet) else {
			if unresolved_pallets.contains(&f.pallet) {
				skipped.push(format!(
					"{}.{} — `{}` is not a local pallet (upstream Substrate/FRAME crate; \
                     checked at release time by the indexer's own metadata-based check instead)",
					f.pallet, f.item, f.pallet
				));
			}
			continue;
		};
		if !surface.events.contains(&f.item) && !surface.calls.contains(&f.item) {
			dead.push(format!(
				"archive filter {}.{} — neither an event nor a call of `{}` in this repo's \
                 current pallet source (readers: {})",
				f.pallet,
				f.item,
				f.pallet,
				readers_str(&f.readers)
			));
		}
	}

	for s in &referenced.storage_reads {
		let pascal = runtime_pallets
			.iter()
			.find(|(name, _)| lower_first(name) == s.pallet)
			.map(|(name, _)| name.clone());
		let Some(pascal) = pascal else {
			// Same case indexer's own check_names.rs calls "no pallet `X` in the
			// runtime (JS form)" — this is exactly the §19.6 bug class (a
			// snake_case/wrong-cased pallet name that silently fails to resolve).
			dead.push(format!(
				"storage read {}.{} — no pallet's JS name (lowerFirst) equals `{}` \
                 (readers: {})",
				s.pallet,
				s.item,
				s.pallet,
				readers_str(&s.readers)
			));
			continue;
		};
		let Some(surface) = surfaces.get(&pascal) else {
			if unresolved_pallets.contains(&pascal) {
				skipped.push(format!(
					"{}.{} — `{}` is not a local pallet (upstream Substrate/FRAME crate)",
					s.pallet, s.item, pascal
				));
			}
			continue;
		};
		if !surface.storage_js.contains(&s.item) {
			dead.push(format!(
				"storage read {}.{} — not a storage entry of `{}` in this repo's current \
                 pallet source (readers: {})",
				s.pallet,
				s.item,
				pascal,
				readers_str(&s.readers)
			));
		}
	}

	println!(
		"check-referenced-names: {} archive filters + {} storage reads checked against {} \
         local pallet(s) parsed from source ({} pallet(s) skipped as non-local)",
		referenced.archive_filters.len(),
		referenced.storage_reads.len(),
		surfaces.len(),
		unresolved_pallets.len(),
	);

	if !skipped.is_empty() {
		println!("\nSkipped (not local pallets — {} unique):", unresolved_pallets.len());
		for p in &unresolved_pallets {
			println!("  {p}");
		}
	}

	if dead.is_empty() {
		println!("\ncheck-referenced-names: OK — every name resolves in current pallet source.");
		return Ok(());
	}

	println!(
		"\n{} indexer-referenced name(s) do not resolve against this repo's current pallet \
         source — the indexer will fail to decode these once this branch merges and a new \
         metadata artifact is pinned:",
		dead.len()
	);
	for d in &dead {
		// GitHub Actions warning annotation — shows on the PR, never fails the job.
		println!("::warning::check-referenced-names: {d}");
	}

	// WARN-only, deliberately (see the module doc) — never a non-zero exit.
	Ok(())
}
