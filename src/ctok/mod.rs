//! Offline Claude token counts: a port of the counting path of
//! [ctok](https://github.com/sanderland/ctok) (MIT, see `LICENSE`) at commit
//! `8c43ffd3c7be467622a540f025516e49ef78d8b1`, limited to the v4.7 vocabulary under the
//! v4.8+ frame (Claude Opus 4.8, Sonnet 5, Fable 5 and later).

mod stream;
mod unicode;

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;
use serde::de::IgnoredAny;

/// Tokens the v4.8+ frame adds to a single user message.
#[allow(
    dead_code,
    reason = "metering stores content tokens; kept for whole-message counts"
)]
pub const MESSAGE_FRAME: usize = 6;

/// Content tokens of `text` as a single user message, excluding [`MESSAGE_FRAME`].
pub fn content_tokens(text: &str) -> usize {
    count(text, false)
}

/// `frame_bow` selects the v4.7 frame head (used by the fixtures) instead of v4.8+.
fn count(text: &str, frame_bow: bool) -> usize {
    let model = model();
    let norm = stream::nfc(text);
    let tail = norm.iter().rev().take_while(|&&c| c == '\n').count();
    let marked = stream::stream_norm(&norm, frame_bow, text.starts_with(' '));
    model.tile(&marked, |c| model.char_cost(c)) + model.frame_tail(tail)
}

#[derive(Deserialize)]
struct Document {
    tokens: HashMap<String, HashMap<String, IgnoredAny>>,
}

static PIECES: &str = include_str!("pieces_v4_7.json");

struct Model {
    trie: Trie,
    unit_pieces: HashSet<char>,
    byte_tokens: HashSet<Vec<u8>>,
    max_byte_token: usize,
}

fn document() -> Document {
    serde_json::from_str(PIECES).expect("valid ctok vocabulary")
}

fn model() -> &'static Model {
    static MODEL: OnceLock<Model> = OnceLock::new();
    MODEL.get_or_init(|| Model::new(document()))
}

impl Model {
    fn new(doc: Document) -> Self {
        let mut trie = Trie::default();
        let mut unit_pieces = HashSet::new();
        for (group, pieces) in &doc.tokens {
            if group == "bytes_fallback" {
                continue;
            }
            for piece in pieces.keys() {
                let piece = parse_marked(piece);
                if let [c] = piece[..]
                    && !stream::is_marker(c)
                {
                    unit_pieces.insert(c);
                }
                trie.insert(&piece);
            }
        }
        for contraction in doc.tokens["contractions"].keys() {
            let mut piece = parse_marked(contraction);
            piece.push(stream::EOW);
            trie.insert(&piece);
        }
        let mut byte_tokens: HashSet<Vec<u8>> = doc.tokens["bytes_fallback"]
            .keys()
            .map(|hex| {
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex byte token"))
                    .collect()
            })
            .collect();
        byte_tokens.extend(
            unit_pieces
                .iter()
                .map(|c| c.encode_utf8(&mut [0; 4]).as_bytes().to_vec()),
        );
        let max_byte_token = byte_tokens.iter().map(Vec::len).max().unwrap_or(1);
        Self {
            trie,
            unit_pieces,
            byte_tokens,
            max_byte_token,
        }
    }

    /// A character no piece covers: a marker is a token, anything else tiles its UTF-8 bytes
    /// over the byte-prefix tokens, every single byte costing 1.
    fn char_cost(&self, c: char) -> usize {
        if stream::is_marker(c) {
            return 1;
        }
        let c = stream::unescape(c);
        if self.unit_pieces.contains(&c) {
            return 1;
        }
        let mut buf = [0; 4];
        let bytes = c.encode_utf8(&mut buf).as_bytes();
        let mut best = [usize::MAX; 5];
        best[0] = 0;
        for end in 1..=bytes.len() {
            for start in end.saturating_sub(self.max_byte_token)..end {
                if end - start == 1 || self.byte_tokens.contains(&bytes[start..end]) {
                    best[end] = best[end].min(best[start] + 1);
                }
            }
        }
        best[bytes.len()]
    }

    /// Minimum-cost tiling over the cost-1 vocabulary plus a one-character floor.
    fn tile(&self, s: &[char], floor: impl Fn(char) -> usize) -> usize {
        let mut best = vec![0; s.len() + 1];
        for end in 1..=s.len() {
            let node = self.trie.child(0, s[end - 1]);
            best[end] = best[end - 1]
                + match node {
                    Some(n) if self.trie.nodes[n].terminal => 1,
                    _ => floor(s[end - 1]),
                };
            let Some(mut node) = node else { continue };
            for start in (0..end - 1).rev() {
                match self.trie.child(node, s[start]) {
                    Some(n) => node = n,
                    None => break,
                }
                if self.trie.nodes[node].terminal {
                    best[end] = best[end].min(best[start] + 1);
                }
            }
        }
        best[s.len()]
    }

    /// The frame appends ⏎⏎ after the content and one token can span into it, so a final run of
    /// `n` newlines costs the tiling of `n + 2` newlines less the frame's own token.
    fn frame_tail(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        self.tile(&vec!['\n'; n + 2], |_| 1) - 1
    }
}

/// The vocabulary stored backwards: the DP ends a tile at each position and walks starts
/// right to left.
#[derive(Default)]
struct Trie {
    nodes: Vec<Node>,
}

#[derive(Default)]
struct Node {
    terminal: bool,
    children: Vec<(char, usize)>,
}

impl Trie {
    fn insert(&mut self, piece: &[char]) {
        if self.nodes.is_empty() {
            self.nodes.push(Node::default());
        }
        let mut node = 0;
        for &c in piece.iter().rev() {
            node = match self.child(node, c) {
                Some(n) => n,
                None => {
                    let n = self.nodes.len();
                    self.nodes.push(Node::default());
                    let children = &mut self.nodes[node].children;
                    let at = children.partition_point(|&(k, _)| k < c);
                    children.insert(at, (c, n));
                    n
                }
            };
        }
        self.nodes[node].terminal = true;
    }

    fn child(&self, node: usize, c: char) -> Option<usize> {
        let children = &self.nodes[node].children;
        children
            .binary_search_by_key(&c, |&(k, _)| k)
            .ok()
            .map(|i| children[i].1)
    }
}

/// A vocabulary key in ctok's public notation (`⟨bow⟩the⟨eow⟩`, `⟨0xE2⟩`) as marked chars.
fn parse_marked(public: &str) -> Vec<char> {
    fn flush(out: &mut Vec<char>, bytes: &mut Vec<u8>) {
        let text = String::from_utf8(std::mem::take(bytes)).expect("utf-8 piece");
        out.extend(text.chars());
    }
    let mut out = Vec::new();
    let mut bytes = Vec::new();
    let mut rest = public;
    while let Some(c) = rest.chars().next() {
        if let Some((atom, after)) = rest.strip_prefix('⟨').and_then(|r| r.split_once('⟩')) {
            let byte = atom
                .strip_prefix("0x")
                .filter(|h| h.len() == 2 && h.bytes().all(|b| b.is_ascii_hexdigit()));
            let marker = match atom {
                "bow" => Some(stream::BOW),
                "eow" => Some(stream::EOW),
                "shift" => Some(stream::SHIFT),
                "caps" => Some(stream::CAPS),
                _ => None,
            };
            if let Some(hex) = byte {
                bytes.push(u8::from_str_radix(hex, 16).unwrap());
                rest = after;
                continue;
            }
            if marker.is_some() || atom == "pad" {
                flush(&mut out, &mut bytes);
                match marker {
                    Some(m) => out.push(m),
                    None => out.extend("⟨pad⟩".chars()),
                }
                rest = after;
                continue;
            }
        }
        flush(&mut out, &mut bytes);
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut out, &mut bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// The v4.7 family's own frame, for its recorded fixtures.
    const V47_FRAME: usize = 11;

    fn v47(text: &str) -> usize {
        count(text, true) + V47_FRAME
    }

    fn v48(text: &str) -> usize {
        content_tokens(text) + MESSAGE_FRAME
    }

    #[test]
    fn ctok_vocabulary_matches_the_ported_family() {
        let raw: serde_json::Value = serde_json::from_str(PIECES).unwrap();
        assert_eq!(raw["meta"]["message_overhead"], V47_FRAME);
        assert_eq!(raw["meta"]["fold_quotes"], false);
        assert!(raw["meta"]["allcaps_min"].is_null());
        let doc = document();
        let pieces: usize = doc.tokens.values().map(HashMap::len).sum();
        assert_eq!(pieces - doc.tokens["bytes_fallback"].len(), 15283 - 468);
    }

    fn gunzip(bytes: &[u8]) -> String {
        let mut out = String::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut out)
            .unwrap();
        out
    }

    fn fixture(rows: &[u8], counts: &str, key: &str) -> (usize, Vec<String>) {
        let counts: serde_json::Value = serde_json::from_str(counts).unwrap();
        let counts = &counts["v4.7"];
        assert_eq!(counts["model"], "claude-opus-4-7");
        let mut bad = Vec::new();
        let mut n = 0;
        for line in gunzip(rows).lines().filter(|l| !l.trim().is_empty()) {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            let id = row[key].as_str().unwrap();
            let expected = counts["counts"][id].as_u64().unwrap() as usize;
            let ours = v47(row["text"].as_str().unwrap());
            if ours != expected {
                bad.push(format!("{id}: {ours} != {expected}"));
            }
            n += 1;
        }
        (n, bad)
    }

    #[test]
    fn ctok_reproduces_multipl_e() {
        let (n, bad) = fixture(
            include_bytes!("testdata/multipl_e.jsonl.gz"),
            include_str!("testdata/multipl_e_counts.json"),
            "lang",
        );
        assert_eq!((n, bad), (22, Vec::<String>::new()));
    }

    #[test]
    fn ctok_reproduces_rosetta_holdout() {
        let (n, bad) = fixture(
            include_bytes!("testdata/rosetta_holdout.jsonl.gz"),
            include_str!("testdata/rosetta_holdout_counts.json"),
            "id",
        );
        assert_eq!((n, bad), (250, Vec::<String>::new()));
    }

    /// Recorded `count_tokens` content costs from ctok's `tests/test_api.py`.
    #[test]
    fn ctok_recorded_v47_costs() {
        let rows: &[(&str, usize)] = &[
            ("'", 1),
            ("'.", 2),
            ("', '", 3),
            ("'d", 3),
            ("'m", 3),
            ("'F", 3),
            ("'First", 3),
            ("a 'b", 4),
            ("a 'x b", 5),
            ("a  'b", 4),
            ("a \"b", 3),
            ("a (b", 3),
            ("a -b", 3),
            ("a ('b", 4),
            ("a '''a", 4),
            ("z '''w", 4),
            ("f's", 2),
            ("x's", 2),
            ("x't", 2),
            ("'s", 2),
            ("'t", 2),
            ("a 's b", 4),
            ("}'s", 3),
            (".'s.", 4),
            (".'re.", 4),
            (".'ve.", 5),
            ("]                 i", 5),
            ("a                 b", 4),
            ("x'll", 3),
            ("we'll", 3),
            ("a'll b", 4),
            ("a 'll b", 5),
            ("'ll", 3),
            ("1'll", 4),
            ("a  'll b", 5),
            ("A'll", 3),
            (".'ll.", 5),
            ("}'ll", 4),
            ("a)'ll b", 6),
            ("x'llo", 4),
            ("x'lls", 4),
            ("x'S", 3),
            ("x'LL", 4),
            ("x'ji", 4),
            ("x'ka", 4),
            ("x'ing", 4),
            ("a ll b", 4),
            ("ll", 2),
            ("1🐫1", 6),
            ("1 🐫1", 7),
            ("1🐫 1", 7),
            ("1 🐫 1", 8),
            ("1😀1", 5),
            ("1 😀1", 6),
            ("1 😀 1", 7),
            ("1←1", 4),
            ("1 ←1", 6),
            ("1← 1", 5),
            ("1 ← 1", 7),
            ("\u{FE0F}", 2),
            ("⚖\u{FE0F}", 5),
            ("⚖\u{FE0E}", 6),
            ("a\u{FE0F}b", 3),
            ("△", 1),
            ("▽", 3),
            ("◇", 3),
            ("║", 2),
            ("a║b", 3),
            ("╔", 3),
            ("a McN b", 4),
            ("a McQ b", 5),
            ("a MdN b", 5),
            ("a neither b", 4),
            ("a zither b", 4),
            ("a wather b", 4),
            ("'./", 1),
            ("a './b", 3),
            ("./", 2),
            ("a КИ b", 5),
            ("a КЭ b", 6),
            ("a ЛИ b", 7),
            ("a МИ b", 5),
            ("a СИ b", 5),
            ("a ПИ b", 6),
        ];
        for &(text, content) in rows {
            assert_eq!(count(text, true), content, "{text:?}");
        }
    }

    #[test]
    fn ctok_v48_frame() {
        assert_eq!(v48("hello, world"), 10);
        assert_eq!(v48("hello, world"), v47("hello, world") - 5);
        assert_eq!(v48("123"), v47("123") - 6);
        assert_eq!(v48("日本"), v47("日本") - 6);
        assert_eq!(v48(" a"), v48("a") + 1);
        assert_eq!(v47(" a"), v47("a"));
        assert_eq!(v48("hello"), v47("hello") - 5);
        assert_eq!(v48(&format!("hello{}", "\n".repeat(29))), v48("hello") + 1);
        for text in ["\u{F8FF}", "\x01", "\u{E000}\x01\u{85}"] {
            assert_eq!(v48(text), 6);
            assert_eq!(v47(text), 12);
        }
        assert_eq!(v48("a\u{F0B7}b"), v48("ab"));
        assert_eq!(v48("\u{F0000}"), 6 + 4);
        assert_eq!(v48("\u{F0000}\u{F0001}"), 6 + 8);
    }

    /// Single-message `input_tokens` measured on claude-fable-5 through OpenRouter.
    #[test]
    fn ctok_matches_fable_5_baselines() {
        let texts = crate::authenticity::defaults::TOKEN;
        assert_eq!(v48(texts[0]), 27);
        assert_eq!(v48(texts[1]), 68);
    }

    #[test]
    #[ignore]
    fn ctok_bench_150k_chars() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/metering.rs"))
                .unwrap();
        let text: String = source.chars().cycle().take(150_000).collect();
        model();
        let started = std::time::Instant::now();
        let tokens = content_tokens(&text);
        println!(
            "ctok: 150000 chars -> {tokens} tokens in {:?}",
            started.elapsed()
        );
    }
}
