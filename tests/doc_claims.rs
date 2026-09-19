//! CLAUDE.md, checked against the crate it describes.
//!
//! An integration test on purpose: anything under `src/` would be counted by
//! its own scans — a `#[cfg(test)]` block added here to measure which modules
//! carry tests promptly made this module one of them, and the line-count
//! figures would drift by however long this file happens to be.
//!
//! Deliberately MIRRORED per crate rather than shared from a helper: each is
//! its own git repository that must build standalone, and this needs nothing
//! but std.

/// CLAUDE.md's `(~Nk lines)` figures, checked against the files they describe.
///
/// Those numbers exist to set expectations before opening a file — "this is
/// the big one" — and they drift in total silence, because a stale number
/// reads exactly like a fresh one. A workspace sweep on 2026-09-19 found
/// EVERY size figure in every CLAUDE.md stale, all of them undercounts, the
/// worst by 49% (cce-designer's app.rs, written ~5.4k at 8046 lines).
///
/// Tolerance is 10%: loose enough that ordinary work does not trip it, tight
/// enough that a file cannot quietly double. When it fails, write the number
/// it reports — that is the whole fix.
///
/// Deliberately MIRRORED into each crate that carries such a figure rather
/// than shared from a helper: every crate here is its own git repository and
/// must build standalone, and this needs nothing but `std`. Same call
/// `ramp.rs` makes about its cce-ui parser.
pub mod size_claims {
    use std::path::{Path, PathBuf};

    /// One `(~Nk lines)` claim: the name as CLAUDE.md spells it, the figure,
    /// and whether the claim also calls it the largest file.
    fn claims(doc: &str) -> Vec<(String, f64, bool)> {
        const TAIL: &str = " lines)";
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(p) = doc[i..].find(TAIL) {
            let end = i + p;
            i = end + TAIL.len();
            let Some(open) = doc[..end].rfind('(') else { continue };
            let inner = &doc[open + 1..end];
            // "~8k", or "largest file, ~6.9k" — take the last word.
            let largest = inner.contains("largest file");
            let word = inner.rsplit([' ', ',']).next().unwrap_or("").trim();
            let digits = word.trim_start_matches('~');
            let value = match digits.strip_suffix('k') {
                Some(k) => k.parse::<f64>().ok().map(|v| v * 1000.0),
                None => digits.parse::<f64>().ok(),
            };
            // The backticked name immediately before the parenthetical.
            let before = &doc[..open];
            let name = before.rfind('`').and_then(|e| {
                before[..e].rfind('`').map(|s| before[s + 1..e].to_string())
            });
            if let (Some(v), Some(n)) = (value, name) {
                if v > 0.0 {
                    out.push((n, v, largest));
                }
            }
        }
        out
    }

    /// Every `.rs` file under `src/`, as (path, line count).
    fn sources(root: &Path) -> Vec<(PathBuf, usize)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, usize)>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    if let Ok(s) = std::fs::read_to_string(&p) {
                        out.push((p, s.lines().count()));
                    }
                }
            }
        }
        let mut v = Vec::new();
        walk(&root.join("src"), &mut v);
        v
    }

    #[test]
    fn test_claude_md_line_counts_match_the_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let doc = std::fs::read_to_string(root.join("CLAUDE.md"))
            .expect("CLAUDE.md is missing next to Cargo.toml");
        let files = sources(root);
        let claims = claims(&doc);
        assert!(
            !claims.is_empty(),
            "no `(~N lines)` figure found in CLAUDE.md — either the syntax changed \
             and this scan needs updating, or the figures were removed and so \
             should this test"
        );

        let mut bad: Vec<String> = Vec::new();
        for (name, claimed, largest) in &claims {
            // A path relative to the crate root, else a unique basename.
            let hits: Vec<&(PathBuf, usize)> = if root.join(name).is_file() {
                files.iter().filter(|(p, _)| *p == root.join(name)).collect()
            } else {
                files
                    .iter()
                    .filter(|(p, _)| p.file_name().and_then(|x| x.to_str()) == Some(name.as_str()))
                    .collect()
            };
            let [(path, actual)] = hits[..] else {
                bad.push(format!("`{name}`: names {} files under src/, cannot check", hits.len()));
                continue;
            };
            let actual = *actual as f64;
            let drift = (actual - claimed) / claimed;
            if drift.abs() > 0.10 {
                bad.push(format!(
                    "`{name}` is documented as ~{} lines but has {} ({:+.0}%) — write ~{}",
                    round_k(*claimed),
                    actual as usize,
                    drift * 100.0,
                    round_k(actual)
                ));
            }
            if *largest {
                if let Some((big, n)) = files.iter().max_by_key(|(_, n)| *n) {
                    if big != path {
                        bad.push(format!(
                            "`{name}` is called the largest file, but {} has {n} lines",
                            big.strip_prefix(root).unwrap_or(big).display()
                        ));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "CLAUDE.md size claims are stale:\n  {}", bad.join("\n  "));
    }

    /// "~8k" for 8046, "~3.1k" for 3134, "~950" for 950 — the spelling the
    /// docs already use, so the failure message can be pasted straight in.
    fn round_k(n: f64) -> String {
        if n < 1000.0 {
            return format!("{}", n.round() as usize);
        }
        let k = n / 1000.0;
        if (k - k.round()).abs() < 0.05 {
            format!("{}k", k.round() as usize)
        } else {
            format!("{k:.1}k")
        }
    }
}

/// CLAUDE.md's claims about this crate's OWN tests, checked against the tree.
///
/// A test count is the most inviting kind of stale fact: it is concrete, it
/// reads as verified, and it is wrong the moment anyone adds a test. The
/// 2026-09-19 sweep found cce-window-manager documented at "~143 unit tests"
/// with 175, and its list of test-free modules naming `state.rs`, which had
/// grown tests — the list was right when written and had quietly inverted.
///
/// Four claim shapes are understood, and each is checked only if the doc
/// actually makes it:
///
/// - `<N> modules carry unit tests` / `<N> modules have them` — how many
///   files under src/ contain a `#[cfg(test)]` block.
/// - `<N> tests` / `~<N> unit tests` — total `#[test]` count. A `~` figure
///   gets a 10% tolerance; a bare one must be exact, because that is what
///   writing a bare number claims.
/// - ``  `x.rs` (the most  `` / `` `x.rs` has the most `` — that file has the
///   most `#[test]`s of any.
/// - `every module has one except `a.rs` … and `b.rs`` — exactly those lack
///   tests. Where the doc instead LISTS the modules that have them, the list
///   must match the real set exactly.
///
/// Deliberately MIRRORED per crate rather than shared: each is its own git
/// repository and must build standalone, and this needs nothing but std.
pub mod test_claims {
    use std::path::{Path, PathBuf};

    /// Every `.rs` under src/ that carries a `#[cfg(test)]` block, with how
    /// many `#[test]` functions it holds.
    fn test_modules(root: &Path) -> Vec<(PathBuf, usize)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, usize)>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    if let Ok(s) = std::fs::read_to_string(&p) {
                        if s.contains("#[cfg(test)]") {
                            out.push((p, s.matches("#[test]").count()));
                        }
                    }
                }
            }
        }
        let mut v = Vec::new();
        walk(&root.join("src"), &mut v);
        v.sort();
        v
    }

    /// Every `.rs` under src/, tests or not — the denominator for "every
    /// module has one except …".
    fn all_sources(root: &Path) -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }
        let mut v = Vec::new();
        walk(&root.join("src"), &mut v);
        v.sort();
        v
    }

    fn word_to_num(w: &str) -> Option<usize> {
        const WORDS: [&str; 21] = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
            "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen",
            "seventeen", "eighteen", "nineteen", "twenty",
        ];
        // Docs wrap these in punctuation — "(nine", "(~175", "48 tests:" —
        // and a token that fails to parse makes the claim silently unchecked,
        // which is the failure this whole file exists to prevent.
        let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if let Ok(n) = w.parse::<usize>() {
            return Some(n);
        }
        let lower = w.to_ascii_lowercase();
        WORDS.iter().position(|x| *x == lower)
    }

    /// The token right before `phrase`, and whether it was written with `~`.
    fn number_before(doc: &str, phrase: &str) -> Option<(usize, bool)> {
        let at = doc.find(phrase)?;
        let word = doc[..at].split_whitespace().next_back()?;
        Some((word_to_num(word)?, word.contains('~')))
    }

    /// Backticked `*.rs` names from `phrase` up to the end of its sentence.
    fn names_after(doc: &str, phrase: &str) -> Vec<String> {
        let Some(at) = doc.find(phrase) else { return Vec::new() };
        let seg = &doc[at..];
        let end = seg.find(". ").unwrap_or(seg.len());
        seg[..end]
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|t| t.ends_with(".rs"))
            .map(|t| t.rsplit('/').next().unwrap_or(t).to_string())
            .collect()
    }

    fn base(p: &Path) -> String {
        p.file_name().and_then(|x| x.to_str()).unwrap_or_default().to_string()
    }

    #[test]
    fn test_claude_md_test_claims_match_the_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let doc = std::fs::read_to_string(root.join("CLAUDE.md"))
            .expect("CLAUDE.md is missing next to Cargo.toml");
        let mods = test_modules(root);
        let total: usize = mods.iter().map(|(_, n)| n).sum();
        let mut bad: Vec<String> = Vec::new();
        let mut checked = 0usize;

        // --- how many modules carry tests
        for phrase in ["modules carry unit tests", "modules have them", "modules have unit tests"] {
            if let Some((claimed, _)) = number_before(&doc, phrase) {
                checked += 1;
                if claimed != mods.len() {
                    bad.push(format!(
                        "\"{claimed} {phrase}\" — {} modules actually do: {}",
                        mods.len(),
                        mods.iter().map(|(p, _)| base(p)).collect::<Vec<_>>().join(", ")
                    ));
                }
                // Where the doc ENUMERATES them, the list must be the real
                // set. Two or more names means a list; a single one is the
                // "`browse.rs` has the most" shape, checked separately.
                let listed = names_after(&doc, phrase);
                if listed.len() >= 2 {
                    let mut want: Vec<String> = mods.iter().map(|(p, _)| base(p)).collect();
                    let mut got = listed.clone();
                    want.sort();
                    got.sort();
                    got.dedup();
                    if want != got {
                        bad.push(format!(
                            "the listed modules {got:?} are not the ones that have tests {want:?}"
                        ));
                    }
                }
            }
        }

        // --- total test count
        for phrase in ["unit tests", "tests"] {
            if let Some((claimed, approx)) = number_before(&doc, phrase) {
                checked += 1;
                let off = (total as f64 - claimed as f64) / claimed.max(1) as f64;
                let stale = if approx { off.abs() > 0.10 } else { total != claimed };
                if stale {
                    bad.push(format!(
                        "\"{}{claimed} {phrase}\" — the crate has {total}",
                        if approx { "~" } else { "" }
                    ));
                }
                break; // "unit tests" wins; don't double-count its "tests" tail
            }
        }

        // --- which file has the most
        for (name, phrase) in [("(the most", "(the most"), ("has the most", "has the most")] {
            let _ = name;
            if let Some(at) = doc.find(phrase) {
                let before = &doc[..at];
                let claimed = before.rfind('`').and_then(|e| {
                    before[..e].rfind('`').map(|s| before[s + 1..e].to_string())
                });
                if let Some(claimed) = claimed.filter(|c| c.ends_with(".rs")) {
                    checked += 1;
                    let claimed = claimed.rsplit('/').next().unwrap_or(&claimed).to_string();
                    if let Some((top, n)) = mods.iter().max_by_key(|(_, n)| *n) {
                        if base(top) != claimed {
                            bad.push(format!(
                                "`{claimed}` is called the one with the most tests, but {} has {n}",
                                base(top)
                            ));
                        }
                    }
                }
            }
        }

        // --- "every module has one except a.rs and b.rs"
        if doc.contains("every module has one except") {
            checked += 1;
            let claimed = names_after(&doc, "every module has one except");
            let with: Vec<String> = mods.iter().map(|(p, _)| base(p)).collect();
            let mut without: Vec<String> = all_sources(root)
                .iter()
                .map(|p| base(p))
                .filter(|b| !with.contains(b))
                .collect();
            let mut claimed = claimed;
            without.sort();
            without.dedup();
            claimed.sort();
            claimed.dedup();
            if claimed != without {
                bad.push(format!(
                    "the modules without tests are {without:?}, not {claimed:?}"
                ));
            }
        }

        assert!(
            checked > 0,
            "CLAUDE.md makes no test-count claim this scan recognizes — either the \
             wording changed and the scan needs updating, or the claims were removed \
             and so should this test"
        );
        assert!(bad.is_empty(), "CLAUDE.md test claims are stale:\n  {}", bad.join("\n  "));
    }
}
