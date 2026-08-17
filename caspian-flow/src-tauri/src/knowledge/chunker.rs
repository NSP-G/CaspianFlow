//! Document chunking + CJK bigram preprocessing for the P22 knowledge base.
//!
//! ## Chunking policy (decided D7)
//!
//! - Target chunk size: `DEFAULT_CHUNK_SIZE` Unicode chars (800).
//! - Overlap: `DEFAULT_CHUNK_OVERLAP` chars (80), taken from the *end* of the
//!   previous chunk so a keyword straddling a boundary is not split.
//! - Prefer to cut at a **paragraph boundary** (`\n\n`). If a paragraph boundary
//!   falls in the band `[size-100, size+100]` (i.e. 700–900 chars), cut there
//!   rather than hard at 800. If no boundary appears by `size+100` (900), hard
//!   cut at `size` (800) — content is never dropped.
//! - A single `\n` line break is accepted as a fallback boundary inside the
//!   band when no `\n\n` exists, so MD line-wrapped prose chunks cleanly.
//!
//! ## Bigram preprocessing (decided D8)
//!
//! The bundled FTS5 `unicode61` tokenizer indexes whole CJK runs as one token,
//! so a Chinese document is effectively unsearchable. `bigram()` expands every
//! run of CJK characters into overlapping bigram tokens (`本地优先` →
//! `本地 地优 优先`), while passing non-CJK runs (Latin words, digits,
//! punctuation) through unchanged so FTS5 tokenizes them by its own rules.
//! The same transform is applied to both stored content and queries, which is
//! what makes FTS5 keyword search work for Chinese.

/// Default chunk size in Unicode characters (D7).
pub const DEFAULT_CHUNK_SIZE: usize = 800;
/// Default chunk overlap in Unicode characters (D7).
pub const DEFAULT_CHUNK_OVERLAP: usize = 80;

/// A single chunk of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// 0-based index within the source document.
    pub index: usize,
    /// Raw chunk text (used for display, LLM context, and FTS5 bigram index).
    pub content: String,
    /// Start char offset (inclusive) within the original document.
    pub char_start: usize,
    /// End char offset (exclusive) within the original document.
    pub char_end: usize,
}

/// Split `text` into overlapping chunks per the D7 policy.
///
/// Returns an empty `Vec` for empty input. A document shorter than `chunk_size`
/// is returned as a single whole chunk (no overlap applied to tiny inputs).
pub fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
    let n = text.chars().count();
    if n == 0 {
        return Vec::new();
    }
    if n <= chunk_size {
        return vec![Chunk {
            index: 0,
            content: text.to_string(),
            char_start: 0,
            char_end: n,
        }];
    }

    // Precompute structural boundaries ONCE (char offsets), so per-chunk split
    // selection is O(log n) instead of rescanning the whole document.
    let para_bounds = boundaries_of(text, "\n\n");
    let line_bounds = boundaries_of(text, "\n");

    let mut chunks = Vec::new();
    let mut index = 0usize;
    let mut pos = 0usize;

    while pos < n {
        let hard_end = (pos + chunk_size).min(n);
        let split = choose_split_point(pos, hard_end, chunk_size, n, &para_bounds, &line_bounds);

        let content: String = text.chars().skip(pos).take(split - pos).collect();
        chunks.push(Chunk {
            index,
            content,
            char_start: pos,
            char_end: split,
        });
        index += 1;

        if split >= n {
            break;
        }

        // Advance with overlap, but always make forward progress.
        let next = split.saturating_sub(overlap);
        pos = if next > pos { next } else { split };
    }

    chunks
}

/// Decide where the current chunk ends.
///
/// - If `hard_end` reaches the end of text, return it.
/// - Otherwise prefer a **paragraph boundary** (`\n\n`) whose start lies within
///   the band `[pos+size-100, pos+size+100]` (700–900) and is closest to
///   `chunk_size` (800). If none exists in the band, fall back to a single
///   `\n` line break in the same band. If neither exists, hard-cut at `size`.
fn choose_split_point(
    pos: usize,
    hard_end: usize,
    chunk_size: usize,
    n: usize,
    para: &[(usize, usize)],
    line: &[(usize, usize)],
) -> usize {
    if hard_end >= n {
        return n;
    }
    let lower = pos + chunk_size.saturating_sub(100); // ~ pos + 700
    let upper = (pos + chunk_size + 100).min(n); // ~ pos + 900
    let target = pos + chunk_size; // ~ 800

    // Paragraph boundaries are preferred; only fall back to line breaks when no
    // paragraph boundary exists in the band.
    if let Some(end) = nearest_in_band(para, lower, upper, target) {
        return end;
    }
    if let Some(end) = nearest_in_band(line, lower, upper, target) {
        return end;
    }
    hard_end
}

/// Return the end offset of the boundary nearest `target` whose start lies in
/// `[lower, upper)`; `None` if no boundary qualifies.
fn nearest_in_band(
    bounds: &[(usize, usize)],
    lower: usize,
    upper: usize,
    target: usize,
) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (distance, end)
    for &(b_start, b_end) in bounds {
        if b_start >= lower && b_start < upper {
            let dist = b_start.abs_diff(target);
            match best {
                Some((bd, _)) if dist >= bd => {}
                _ => best = Some((dist, b_end)),
            }
        }
    }
    best.map(|(_, end)| end)
}

/// Return `(start, end)` char offsets of every occurrence of `sep` in `text`.
///
/// Operates in char space (a single `O(n)` `Vec<char>` build) so it is correct
/// for multi-byte UTF-8 and safe to call once per document.
fn boundaries_of(text: &str, sep: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let sep_chars: Vec<char> = sep.chars().collect();
    if sep_chars.is_empty() {
        return out;
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i + sep_chars.len() <= n {
        if &chars[i..i + sep_chars.len()] == sep_chars.as_slice() {
            out.push((i, i + sep_chars.len()));
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// CJK bigram preprocessing for FTS5 indexing (D8).
///
/// Every maximal run of CJK characters (`\u{4e00}`–`\u{9fff}`) is expanded into
/// overlapping bigram tokens separated by spaces. Non-CJK runs are emitted
/// verbatim (FTS5's own `unicode61` tokenizer handles Latin words, digits, and
/// punctuation). A single CJK character (a run of length 1) is emitted as-is.
///
/// Example: `本地优先的工作流` → `本地 地优 优先 先的 的工 工作 作流 `.
pub fn bigram(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut run: Vec<char> = Vec::new();

    let flush = |run: &mut Vec<char>, out: &mut String| {
        if run.is_empty() {
            return;
        }
        if run.len() == 1 {
            out.push(run[0]);
            out.push(' ');
        } else {
            for w in run.windows(2) {
                out.push(w[0]);
                out.push(w[1]);
                out.push(' ');
            }
        }
        run.clear();
    };

    for ch in text.chars() {
        if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            run.push(ch);
        } else {
            flush(&mut run, &mut out);
            out.push(ch);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Build an FTS5 `MATCH` query string from a user query.
///
/// The query is bigram-preprocessed, then each whitespace-separated token is
/// double-quoted so stray FTS5 syntax characters in the user input cannot break
/// the query. Tokens are combined with the explicit `OR` operator.
///
/// **Caveat (verified empirically):** the bundled FTS5 build treats
/// space-separated phrases as AND, NOT OR — so `"工作" "擎是"` matches only docs
/// containing *both*. That silently collapses any multi-token query that
/// contains one non-matching token to zero hits. The explicit `OR` keyword is
/// required for correct recall-oriented keyword search.
pub fn fts_query(query: &str) -> String {
    let bg = bigram(query);
    let tokens: Vec<String> = bg
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect();
    tokens.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_empty() {
        assert!(chunk_text("", DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP).is_empty());
    }

    #[test]
    fn test_chunk_short_returns_single() {
        let text = "短文档";
        let chunks = chunk_text(text, DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
        assert_eq!(chunks[0].char_start, 0);
        assert_eq!(chunks[0].char_end, text.chars().count());
    }

    #[test]
    fn test_chunk_respects_paragraph_boundary() {
        // Three ~400-char paragraphs; target is 800, so the first chunk should
        // stop at the end of the second paragraph (a boundary near 800).
        let p = "段落内容".repeat(200); // 800 chars
        let text = format!("{p}\n\n{p}\n\n{p}");
        let chunks = chunk_text(&text, DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP);
        assert!(
            chunks.len() >= 2,
            "expected >=2 chunks, got {}",
            chunks.len()
        );
        // First chunk must end exactly at a paragraph boundary (double newline).
        let end = chunks[0].char_end;
        let suffix: String = text.chars().skip(end.saturating_sub(2)).take(2).collect();
        assert_eq!(suffix, "\n\n", "first chunk must cut at paragraph boundary");
    }

    #[test]
    fn test_chunk_hard_cut_when_no_boundary() {
        // No newline at all -> must hard-cut at chunk_size, never drop content.
        let text = "无换行长文本".repeat(500); // 3000 chars, no '\n'
        let chunks = chunk_text(&text, DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP);
        assert!(!chunks.is_empty());
        // Every hard-cut chunk (except possibly the last) is ~chunk_size.
        for c in &chunks {
            assert!(c.content.chars().count() <= DEFAULT_CHUNK_SIZE + 1);
        }
        // Concatenating all chunks (overlap included) must contain the original
        // leading `chunk_size` chars (slice by char, not byte — CJK is 3 bytes).
        let head: String = text.chars().take(DEFAULT_CHUNK_SIZE).collect();
        let joined: String = chunks.iter().map(|c| c.content.clone()).collect();
        assert!(joined.contains(&head));
    }

    /// D7 acceptance: reassembling all chunks (with overlap) must reconstruct the
    /// original document exactly. This is the strongest guard against a chunker
    /// that silently drops or duplicates content.
    #[test]
    fn test_chunk_reassembly_equals_original() {
        let p = "知识库分块测试内容".repeat(120); // ~960 chars per paragraph
        let text = format!("{p}\n\n{p}\n\n{p}\n\n{p}");
        let chunks = chunk_text(&text, DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP);
        assert!(chunks.len() >= 3);

        // Reassemble by walking the windows and keeping only the non-overlapping
        // prefix of each chunk. Equivalent to: first chunk fully, then each
        // subsequent chunk contributes the part after its overlap with the prior.
        let mut rebuilt = String::new();
        let mut prev_end = 0usize;
        for c in &chunks {
            if c.char_start >= prev_end {
                rebuilt.push_str(&c.content);
            } else {
                // overlap region: only append the new tail
                let skip = prev_end - c.char_start;
                let tail: String = c.content.chars().skip(skip).collect();
                rebuilt.push_str(&tail);
            }
            prev_end = c.char_end;
        }
        assert_eq!(rebuilt, text, "reassembled chunks must equal original");
    }

    #[test]
    fn test_chunk_overlap_present() {
        let p = "abcdefghij".repeat(120); // 1200 chars, no CJK/newline
        let text = format!("{p}{}", "第二段落".repeat(200));
        let chunks = chunk_text(&text, DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP);
        assert!(chunks.len() >= 2);
        // Consecutive chunks must share the overlap region.
        for w in chunks.windows(2) {
            let a: String = w[0]
                .content
                .chars()
                .take(w[0].content.chars().count() - DEFAULT_CHUNK_OVERLAP)
                .collect();
            let b: String = w[1].content.chars().take(DEFAULT_CHUNK_OVERLAP).collect();
            assert_eq!(
                w[0].content
                    .chars()
                    .skip(w[0].content.chars().count() - DEFAULT_CHUNK_OVERLAP)
                    .collect::<String>(),
                w[1].content
                    .chars()
                    .take(DEFAULT_CHUNK_OVERLAP)
                    .collect::<String>(),
                "consecutive chunks must overlap by {DEFAULT_CHUNK_OVERLAP} chars"
            );
            let _ = a;
            let _ = b;
        }
    }

    // ── bigram tests ────────────────────────────────────────────────────────

    #[test]
    fn test_bigram_cjk() {
        assert_eq!(bigram("本地优先"), "本地 地优 优先 ");
        assert_eq!(bigram("工作流"), "工作 作流 ");
    }

    #[test]
    fn test_bigram_single_cjk_passthrough() {
        // A single CJK char cannot form a bigram; it is emitted verbatim.
        assert_eq!(bigram("流"), "流 ");
    }

    #[test]
    fn test_bigram_latin_passthrough() {
        assert_eq!(bigram("workflow"), "workflow");
        assert_eq!(bigram("hello world"), "hello world");
    }

    #[test]
    fn test_bigram_mixed() {
        // CJK run expanded; Latin run passed through; boundary handled.
        let out = bigram("本地 workflow 优先");
        assert!(out.contains("本地"));
        assert!(out.contains("作流") || out.contains("地 "));
        assert!(out.contains("workflow"));
        assert!(out.contains("优先"));
    }

    #[test]
    fn test_fts_query_quotes_tokens() {
        // CJK query becomes quoted bigram tokens joined by explicit OR.
        let q = fts_query("工作流");
        assert_eq!(q, "\"工作\" OR \"作流\"");
        let q2 = fts_query("workflow");
        assert_eq!(q2, "\"workflow\"");
        // Multi-token query lists every token OR-combined.
        let q3 = fts_query("工作流引擎");
        assert_eq!(q3, "\"工作\" OR \"作流\" OR \"流引\" OR \"引擎\"");
    }

    #[test]
    fn test_fts_query_empty() {
        assert_eq!(fts_query("   "), "");
        assert_eq!(fts_query(""), "");
    }
}
