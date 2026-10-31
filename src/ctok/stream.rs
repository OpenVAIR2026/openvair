//! Text to the marked stream: ctok's `normalize.py` for the v4.7 vocabulary, whose families
//! fold no quotes and write no ⟨caps⟩.

use unicode_normalization::{UnicodeNormalization, char::canonical_combining_class};

use super::unicode::{Gc, category};

pub const BOW: char = '\u{FDD0}';
pub const EOW: char = '\u{FDD1}';
pub const SHIFT: char = '\u{FDD3}';
pub const CAPS: char = '\u{FDD4}';

pub fn is_marker(c: char) -> bool {
    matches!(c, BOW | EOW | SHIFT | CAPS)
}

/// Literal marker codepoints in the input are escaped to these private-use codepoints after
/// NFC has stripped every private-use character, and map back for pricing.
pub fn unescape(c: char) -> char {
    match c {
        '\u{E000}'..='\u{E004}' => char::from_u32(0xFDD0 + (c as u32 - 0xE000)).unwrap(),
        _ => c,
    }
}

fn escape(c: char) -> char {
    match c {
        '\u{FDD0}'..='\u{FDD4}' => char::from_u32(0xE000 + (c as u32 - 0xFDD0)).unwrap(),
        _ => c,
    }
}

/// NFC plus the measured strips and folds.
pub fn nfc(text: &str) -> Vec<char> {
    let chars: Vec<char> = text
        .nfc()
        .filter(|&c| !matches!(c, '\x01'..='\x08' | '\x0B'..='\x1F' | '\x7F'..='\u{9F}'))
        .map(|c| if c == '\0' { ' ' } else { c })
        .collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if c == '\u{0E4D}' && chars.get(i) == Some(&'\u{0E32}') {
            out.push('\u{0E33}');
            i += 1;
        } else if !('\u{E000}'..='\u{F8FF}').contains(&c) {
            out.push(match c {
                '\u{A0}'
                | '\u{1680}'
                | '\u{2000}'..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}' => ' ',
                _ => c,
            });
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Wordy,
    Hard,
    Digit,
    Punct,
    Space,
    StrayMark,
}

const NEW_CASE_PAIRS: [(char, char); 2] = [('\u{1C89}', '\u{1C8A}'), ('\u{A7CB}', '\u{0264}')];
const UNCASED: char = '\u{03F4}';
const PUNCT_SYMS: &str = "—»«•°„–−£§€…√→（№†└│།·─═█";
const IDEOGRAPHIC_SYMBOLS: &str = "〄〒〓〠〶〷〾〿";
const SYMBOL_LETTERS: [(u32, u32); 4] = [
    (0x16EE, 0x16F0),
    (0x2160, 0x2188),
    (0x24B6, 0x24E9),
    (0xA6E6, 0xA6EF),
];
const CONTRACTION_SUFFIXES: [&str; 7] = ["s", "t", "d", "m", "ll", "re", "ve"];

fn is_new_cased(c: char) -> bool {
    NEW_CASE_PAIRS.iter().any(|&(u, l)| c == u || c == l)
}

fn push_lower(out: &mut Vec<char>, c: char) {
    if c == UNCASED {
        out.push(c);
    } else if let Some(&(_, l)) = NEW_CASE_PAIRS.iter().find(|&&(u, _)| u == c) {
        out.push(l);
    } else {
        out.extend(c.to_lowercase());
    }
}

fn lowers_to_itself(c: char) -> bool {
    let mut lower = Vec::with_capacity(1);
    push_lower(&mut lower, c);
    lower == [c]
}

fn is_upper(c: char) -> bool {
    c != UNCASED
        && (NEW_CASE_PAIRS.iter().any(|&(u, _)| u == c)
            || (c.is_uppercase() && !lowers_to_itself(c)))
}

fn is_mark(c: char) -> bool {
    category(c) == Gc::M
}

fn is_selector(c: char) -> bool {
    ('\u{FE00}'..='\u{FE0F}').contains(&c)
}

fn is_hard_cp(o: u32) -> bool {
    o >= 0x10000
        || (0x4E00..=0x9FFF).contains(&o)
        || (0x3400..=0x4DBF).contains(&o)
        || (0xF900..=0xFAFF).contains(&o)
        || (0xAC00..=0xD7A3).contains(&o)
        || matches!(
            o,
            0x3005 | 0x3006 | 0x3031 | 0x3032 | 0x3033 | 0x3034 | 0x3035 | 0x303B | 0x303C
        )
}

fn classify(c: char) -> Class {
    if is_new_cased(c) {
        return Class::Wordy;
    }
    let o = c as u32;
    let cat = category(c);
    if cat == Gc::Z || matches!(c, '\t' | '\n' | '\r' | '\x0C' | '\x0B') {
        return Class::Space;
    }
    if SYMBOL_LETTERS
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&o))
    {
        return Class::Wordy;
    }
    if cat == Gc::Nd
        && (o < 0x80 || (0x0660..=0x0669).contains(&o) || (0x06F0..=0x06F9).contains(&o))
    {
        return Class::Digit;
    }
    if (o < 0x80 && matches!(cat, Gc::P | Gc::S)) || PUNCT_SYMS.contains(c) {
        return Class::Punct;
    }
    if is_selector(c) {
        return Class::Hard;
    }
    if cat == Gc::M {
        return if o < 0x10000 && c.is_alphabetic() {
            Class::Wordy
        } else {
            Class::Hard
        };
    }
    if cat == Gc::L && !is_hard_cp(o) {
        Class::Wordy
    } else {
        Class::Hard
    }
}

/// The v4.7 case marking: ⟨shift⟩ for a title-case span, everything else literal.
fn mark_case(span: &[char], head_mark: bool) -> Vec<char> {
    if head_mark || span.contains(&'ẞ') {
        return span.to_vec();
    }
    let (first, tail) = (span[0], &span[1..]);
    if span.contains(&'İ') {
        if is_upper(first) && first != 'İ' && !tail.iter().any(|&c| c != 'İ' && is_upper(c)) {
            let mut out = vec![SHIFT];
            push_lower(&mut out, first);
            for &c in tail {
                if c == 'İ' {
                    out.push(c);
                } else {
                    push_lower(&mut out, c);
                }
            }
            return out;
        }
        return span.to_vec();
    }
    if lowers_to_itself(first) || !is_upper(first) || tail.iter().any(|&c| is_upper(c)) {
        return span.to_vec();
    }
    let mut out = vec![SHIFT];
    for &c in span {
        push_lower(&mut out, c);
    }
    out
}

fn is_separator(c: char) -> bool {
    (c as u32) < 0x10000 && is_mark(c) && !is_selector(c) && !c.is_alphabetic()
}

fn syriac_vowel(c: char) -> bool {
    c == '\u{0711}' || ('\u{0730}'..='\u{073F}').contains(&c)
}

fn stray_mark(c: char) -> bool {
    !syriac_vowel(c) && canonical_combining_class(c) != 0 && !is_separator(c)
}

fn marks_like_punct(c: char) -> bool {
    let c = unescape(c);
    let o = c as u32;
    if o >= 0x10000 {
        return false;
    }
    if is_separator(c) {
        return true;
    }
    let cat = category(c);
    let ideographic_punct =
        (0x3001..=0x303F).contains(&o) && (cat == Gc::P || IDEOGRAPHIC_SYMBOLS.contains(c));
    matches!(cat, Gc::P | Gc::S | Gc::Cf | Gc::Cn) && !ideographic_punct
}

fn digit_border(c: char) -> bool {
    let cat = category(c);
    (cat == Gc::No || (cat == Gc::Nd && !c.is_ascii())) && (c as u32) < 0x10000
}

fn digit_run(body: &[char]) -> bool {
    let Some(&first) = body.first() else {
        return false;
    };
    let cat = category(first);
    matches!(cat, Gc::Nd | Gc::No) && body[1..].iter().all(|&c| category(c) == cat)
}

fn digit_bow(body: &[char]) -> bool {
    digit_run(body) && digit_border(body[0])
}

fn digit_eow(body: &[char]) -> bool {
    digit_run(body) && digit_border(body[body.len() - 1])
}

fn hard_bow(body: &[char]) -> bool {
    body.first()
        .is_some_and(|&c| is_selector(c) || marks_like_punct(c))
}

fn hard_eow(body: &[char]) -> bool {
    body.last()
        .is_some_and(|&c| is_selector(c) || marks_like_punct(c))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HardKind {
    Punct,
    Number,
    Letter,
}

fn hard_kind(c: char) -> HardKind {
    if marks_like_punct(c) {
        HardKind::Punct
    } else if digit_border(c) {
        HardKind::Number
    } else {
        HardKind::Letter
    }
}

type Run<'a> = (Class, &'a [char]);

fn runs(norm: &[char]) -> Vec<Run<'_>> {
    let mut out = Vec::new();
    let Some(&head) = norm.first() else {
        return out;
    };
    let mut start = 0;
    let mut cur = classify(head);
    if cur == Class::Wordy && stray_mark(head) {
        cur = Class::StrayMark;
    }
    for (i, &ch) in norm.iter().enumerate().skip(1) {
        let c = classify(ch);
        if (cur == Class::StrayMark && c == Class::Wordy && stray_mark(ch)) || c == cur {
            continue;
        }
        out.push((cur, &norm[start..i]));
        start = i;
        cur = if c == Class::Wordy && stray_mark(ch) && cur != Class::Wordy {
            Class::StrayMark
        } else {
            c
        };
    }
    out.push((cur, &norm[start..]));

    // HARD runs split where the pretoken kind changes; a variation selector rides its base.
    let mut split = Vec::with_capacity(out.len());
    for (cls, body) in out {
        if cls != Class::Hard || body.len() == 1 {
            split.push((cls, body));
            continue;
        }
        let mut start = 0;
        let mut kind = hard_kind(body[0]);
        for (i, &ch) in body.iter().enumerate().skip(1) {
            if is_selector(ch) || hard_kind(ch) == kind {
                continue;
            }
            split.push((cls, &body[start..i]));
            start = i;
            kind = hard_kind(ch);
        }
        split.push((cls, &body[start..]));
    }
    split
}

fn opens_word(runs: &[Run], i: usize) -> bool {
    runs[i].1 == ['\'']
        && runs
            .get(i + 1)
            .is_some_and(|r| matches!(r.0, Class::Wordy | Class::StrayMark))
}

fn takes_right_border((cls, body): Run) -> bool {
    cls == Class::Punct
        || hard_eow(body)
        || (matches!(cls, Class::Digit | Class::Hard) && digit_run(body) && digit_eow(body))
}

fn contraction_seam(runs: &[Run], i: usize) -> bool {
    let body: String = runs[i].1.iter().collect();
    if i == 0 || !CONTRACTION_SUFFIXES.contains(&body.as_str()) {
        return false;
    }
    if runs[i - 1] != (Class::Punct, &['\''][..]) {
        return false;
    }
    i < 2 || !takes_right_border(runs[i - 2])
}

/// The marked stream over normalized text. `frame_bow` is whether the message frame ends in a
/// ⟨bow⟩ (v4.7) or not (v4.8+); `raw_head_space` is whether the raw text started with a space.
pub fn stream_norm(norm: &[char], frame_bow: bool, raw_head_space: bool) -> Vec<char> {
    let mut norm: Vec<char> = norm.iter().map(|&c| escape(c)).collect();
    while norm.last() == Some(&'\n') {
        norm.pop();
    }
    if frame_bow && raw_head_space && norm.first() == Some(&' ') && norm.get(1) != Some(&' ') {
        norm.remove(0);
    }
    let runs = runs(&norm);
    if runs.is_empty() {
        return if frame_bow { vec![BOW] } else { Vec::new() };
    }
    let n_runs = runs.len();
    let borders_space = |i: usize, left: bool| -> bool {
        let neighbour = if left {
            match i.checked_sub(1) {
                Some(j) => runs[j],
                None => return frame_bow,
            }
        } else {
            match runs.get(i + 1) {
                Some(&r) => r,
                None => return false,
            }
        };
        if neighbour.0 != Class::Space {
            return false;
        }
        if left {
            neighbour.1.last() == Some(&' ')
        } else {
            neighbour.1.first() == Some(&' ') && neighbour.1.get(1) != Some(&' ')
        }
    };

    let (first_cls, first_body) = runs[0];
    let head_quote = opens_word(&runs, 0);
    let has_own_bow = !head_quote
        && (matches!(first_cls, Class::Wordy | Class::Punct | Class::StrayMark)
            || hard_bow(first_body)
            || (matches!(first_cls, Class::Digit | Class::Hard) && digit_bow(first_body))
            || (first_cls == Class::Space && first_body[0] == ' '));
    let mut out = Vec::with_capacity(norm.len() * 2);
    if !has_own_bow && frame_bow {
        out.push(if head_quote { ' ' } else { BOW });
    }
    for (i, &(cls, body)) in runs.iter().enumerate() {
        match cls {
            Class::Wordy => {
                let fused = i > 0 && runs[i - 1].0 == Class::StrayMark;
                let marked = mark_case(body, fused);
                let (pre, rest) = match marked.first() {
                    Some(&m) if m == SHIFT || m == CAPS => marked.split_at(1),
                    _ => marked.split_at(0),
                };
                out.extend_from_slice(pre);
                if !(fused || contraction_seam(&runs, i)) {
                    out.push(BOW);
                }
                out.extend_from_slice(rest);
                out.push(EOW);
            }
            Class::StrayMark => {
                out.push(BOW);
                out.extend_from_slice(body);
                if !(i + 1 < n_runs && runs[i + 1].0 == Class::Wordy) {
                    out.push(EOW);
                }
            }
            _ if cls == Class::Punct || hard_bow(body) || hard_eow(body) => {
                let punct = cls == Class::Punct;
                if borders_space(i, true) && !opens_word(&runs, i) && (punct || hard_bow(body)) {
                    out.push(BOW);
                }
                out.extend_from_slice(body);
                if borders_space(i, false) && (punct || hard_eow(body)) {
                    out.push(EOW);
                }
            }
            Class::Digit | Class::Hard if digit_run(body) => {
                if digit_bow(body) && borders_space(i, true) {
                    out.push(BOW);
                }
                out.extend_from_slice(body);
                if digit_eow(body) && borders_space(i, false) {
                    out.push(EOW);
                }
            }
            _ => out.extend_from_slice(body),
        }
    }
    seam(&out)
}

/// `(.)⟨eow⟩ [⟨shift⟩⟨caps⟩]*⟨bow⟩` → `\1⟨eow⟩[...]⟨bow⟩`: the single space between two marked
/// spans is not written.
fn seam(s: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        out.push(s[i]);
        if s[i] != '\n' && s.get(i + 1) == Some(&EOW) && s.get(i + 2) == Some(&' ') {
            let mut k = i + 3;
            while k < s.len() && (s[k] == SHIFT || s[k] == CAPS) {
                k += 1;
            }
            if s.get(k) == Some(&BOW) {
                out.push(EOW);
                out.extend_from_slice(&s[i + 3..=k]);
                i = k + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}
