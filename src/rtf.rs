//! Bounded RTF tokenization with scoped destinations, code pages and Unicode.
use anyhow::{Result, ensure};
use encoding_rs::{Encoding, WINDOWS_1252};

#[derive(Clone, Copy)]
struct State {
    skip: bool,
    unicode_fallback: usize,
    encoding: &'static Encoding,
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    state: State,
    stack: Vec<State>,
    fallback: usize,
    high_surrogate: Option<u16>,
    ansi: Vec<u8>,
    output: String,
}

impl Parser<'_> {
    fn flush_ansi(&mut self) {
        self.output
            .push_str(&self.state.encoding.decode(&self.ansi).0);
        self.ansi.clear();
    }
    fn flush_surrogate(&mut self) {
        if self.high_surrogate.take().is_some() {
            self.output.push(char::REPLACEMENT_CHARACTER);
        }
    }
    fn byte(&mut self, byte: u8) {
        if self.state.skip {
            return;
        }
        if self.fallback > 0 {
            self.fallback -= 1;
            return;
        }
        self.flush_surrogate();
        self.ansi.push(byte);
    }
    fn character(&mut self, ch: char) {
        if self.state.skip {
            return;
        }
        if self.fallback > 0 {
            self.fallback -= 1;
            return;
        }
        self.flush_ansi();
        self.flush_surrogate();
        self.output.push(ch);
    }
    fn unicode(&mut self, unit: u16) {
        if self.state.skip {
            return;
        }
        self.flush_ansi();
        match unit {
            0xd800..=0xdbff => {
                self.flush_surrogate();
                self.high_surrogate = Some(unit);
            }
            0xdc00..=0xdfff => {
                let character = self.high_surrogate.take().and_then(|high| {
                    char::from_u32(
                        0x10000 + (((high as u32 - 0xd800) << 10) | (unit as u32 - 0xdc00)),
                    )
                });
                self.output
                    .push(character.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
            _ => {
                self.flush_surrogate();
                self.output
                    .push(char::from_u32(unit as u32).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
        self.fallback = self.state.unicode_fallback;
    }
    fn control(&mut self) {
        let Some(&symbol) = self.bytes.get(self.pos) else {
            return;
        };
        self.pos += 1;
        match symbol {
            b'\\' | b'{' | b'}' => self.byte(symbol),
            b'*' => {
                self.flush_ansi();
                self.state.skip = true;
            }
            b'~' => self.character(' '),
            b'_' => self.character('\u{2011}'),
            b'-' => self.character('\u{00ad}'),
            b'\r' | b'\n' => self.character('\n'),
            b'\'' => {
                if let Some(hex) = self.bytes.get(self.pos..self.pos.saturating_add(2)) {
                    if let Ok(text) = std::str::from_utf8(hex)
                        && let Ok(byte) = u8::from_str_radix(text, 16)
                    {
                        self.byte(byte);
                    }
                    self.pos += 2;
                }
            }
            c if c.is_ascii_alphabetic() => {
                let start = self.pos - 1;
                while self
                    .bytes
                    .get(self.pos)
                    .is_some_and(u8::is_ascii_alphabetic)
                {
                    self.pos += 1;
                }
                let word = &self.bytes[start..self.pos];
                let number_start = self.pos;
                if self.bytes.get(self.pos) == Some(&b'-') {
                    self.pos += 1;
                }
                while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                    self.pos += 1;
                }
                let number = std::str::from_utf8(&self.bytes[number_start..self.pos])
                    .ok()
                    .and_then(|text| text.parse::<i32>().ok());
                if self.bytes.get(self.pos) == Some(&b' ') {
                    self.pos += 1;
                }
                self.word(word, number);
            }
            _ => {}
        }
    }
    fn word(&mut self, word: &[u8], number: Option<i32>) {
        if [
            b"fonttbl".as_slice(),
            b"colortbl",
            b"stylesheet",
            b"info",
            b"pict",
            b"object",
            b"generator",
            b"filetbl",
            b"listtable",
            b"listoverridetable",
            b"rsidtbl",
            b"themedata",
            b"datastore",
            b"fldinst",
        ]
        .contains(&word)
        {
            self.flush_ansi();
            self.state.skip = true;
        }
        if word == b"bin" {
            if let Some(count) = number.filter(|count| *count >= 0) {
                self.pos = self
                    .pos
                    .saturating_add(count as usize)
                    .min(self.bytes.len());
            }
            return;
        }
        if self.state.skip {
            return;
        }
        match word {
            b"uc" => {
                if let Some(count) = number.filter(|count| (0..=32767).contains(count)) {
                    self.state.unicode_fallback = count as usize;
                }
            }
            b"u" => {
                if let Some(unit) = number.filter(|unit| (-32768..=65535).contains(unit)) {
                    self.unicode(unit as u16);
                }
            }
            b"ansicpg" => {
                let label = match number {
                    Some(65001) => "utf-8".to_owned(),
                    Some(932) => "shift_jis".to_owned(),
                    Some(936) => "gbk".to_owned(),
                    Some(949) => "euc-kr".to_owned(),
                    Some(950) => "big5".to_owned(),
                    Some(code) => format!("windows-{code}"),
                    None => String::new(),
                };
                if let Some(encoding) = Encoding::for_label(label.as_bytes()) {
                    self.flush_ansi();
                    self.state.encoding = encoding;
                }
            }
            b"par" | b"line" => self.character('\n'),
            b"tab" => self.character('\t'),
            b"emdash" => self.character('—'),
            b"endash" => self.character('–'),
            b"bullet" => self.character('•'),
            b"lquote" => self.character('‘'),
            b"rquote" => self.character('’'),
            b"ldblquote" => self.character('“'),
            b"rdblquote" => self.character('”'),
            _ => {}
        }
    }
    fn parse(mut self) -> Result<String> {
        while let Some(&byte) = self.bytes.get(self.pos) {
            self.pos += 1;
            match byte {
                b'{' => {
                    self.flush_ansi();
                    self.fallback = 0;
                    ensure!(self.stack.len() < 128, "RTF group nesting limit");
                    self.stack.push(self.state);
                }
                b'}' => {
                    self.flush_ansi();
                    self.fallback = 0;
                    if let Some(state) = self.stack.pop() {
                        self.state = state;
                    }
                }
                b'\\' => self.control(),
                b'\r' | b'\n' | 0 => {}
                _ => self.byte(byte),
            }
        }
        self.flush_ansi();
        self.flush_surrogate();
        Ok(self
            .output
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

pub fn strip_rtf(bytes: &[u8]) -> Result<String> {
    Parser {
        bytes,
        pos: 0,
        state: State {
            skip: false,
            unicode_fallback: 1,
            encoding: WINDOWS_1252,
        },
        stack: Vec::new(),
        fallback: 0,
        high_surrogate: None,
        ansi: Vec::new(),
        output: String::new(),
    }
    .parse()
}
