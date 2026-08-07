//! Expands `#include` and `#define` before lexing.
//!
//! # What this is not
//!
//! It is not a C preprocessor. There is no macro facility, no conditional
//! compilation, and no `#pragma`. `#define` accepts exactly two names:
//!
//! - `#define ENGINE_NUM_STRUCTURES <count>` — recognised and ignored.
//! - `#define ENGINE_STRUCTURE_<n> <typename>` — declares an engine structure
//!   type. This is how `effect`, `event`, `location` and `talent` come to
//!   exist; they are not keywords.
//!
//! Anything else is the error `#define unknown or invalid definition`, and a
//! line that does not hold exactly two whitespace-separated tokens after the
//! directive is `#define syntax`. `#define MY_CONST 42` is rejected. That is
//! not an oversight to fix: accepting it would compile scripts the reference
//! compiler refuses, which is worse than refusing them here.
//!
//! # How directives are recognised
//!
//! The reference compiler runs this line by line inside the token fetch loop.
//! For each line it skips characters outside `0x21..=0x7e` and, if the first
//! printable character is `#`, tests the line against the two directive
//! prefixes. A `#` that starts neither is left for the lexer, which warns
//! `Invalid character '#' (0x23) found in source, ignored` and carries on. That
//! is what a stray TSLPatcher `#2DAMEMORY5` token produces: those must be
//! substituted before the compiler runs.
//!
//! Because the fetch loop skips preprocessing while it is inside a block
//! comment, a `#include` inside `/* ... */` is *not* processed. This module
//! tracks comment state across lines so it behaves the same way.
//!
//! # Include resolution
//!
//! `#include "name"` resolves against the directory of the file being
//! processed, not the working directory and not a search path. The name has its
//! extension stripped and `.nss` appended, so `"foo.nss"` and `"foo"` are the
//! same include. Backslash subpaths such as `"incs\inc_a"` are common in
//! existing mod source and work; forward slashes are accepted too.
//!
//! A file already pulled in is skipped rather than included twice, matched
//! case-insensitively, which is also what stops include cycles.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::{CompileError, Diagnostic, Position, Severity, Span, Stage};

/// How many engine structures can be declared.
///
/// The reference compiler holds them in a fixed array of this size and does not
/// bounds check the index it parses out of `ENGINE_STRUCTURE_<n>`. This
/// implementation reports an error instead of writing past the end.
pub const MAX_ENGINE_STRUCTURES: usize = 16;

/// How deep `#include` may nest before the expander gives up.
///
/// Duplicate suppression already makes cycles impossible, so this only guards
/// against a pathological include tree.
pub const MAX_INCLUDE_DEPTH: usize = 64;

/// The engine structure types a translation unit declared.
///
/// Stock KOTOR's `nwscript.nss` declares four, in this order: `effect`,
/// `event`, `location`, `talent`. NWScript for NWN declares a fifth,
/// `itemproperty`, which KOTOR does not have. Nothing here is hard-coded — the
/// table is whatever the `#define` lines said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStructures {
    names: Vec<Option<String>>,
}

impl EngineStructures {
    /// Builds an empty table.
    pub fn new() -> Self {
        EngineStructures::default()
    }

    /// Declares engine structure `index` with the name `name`.
    ///
    /// Returns false when `index` is past [`MAX_ENGINE_STRUCTURES`]. A name
    /// already registered keeps the index it was first given, which is what the
    /// reference compiler's "add only when not already a reserved word" check
    /// amounts to.
    pub fn define(&mut self, index: u8, name: impl Into<String>) -> bool {
        let index = index as usize;
        if index >= MAX_ENGINE_STRUCTURES {
            return false;
        }
        if self.names.len() <= index {
            self.names.resize(index + 1, None);
        }
        let name = name.into();
        if self.lookup(&name).is_none() {
            self.names[index] = Some(name);
        }
        true
    }

    /// Returns the index declared for `name`, if any. Names are case sensitive.
    pub fn lookup(&self, name: &str) -> Option<u8> {
        self.names
            .iter()
            .position(|slot| slot.as_deref() == Some(name))
            .map(|index| index as u8)
    }

    /// Returns the name declared for `index`, if any.
    pub fn name(&self, index: u8) -> Option<&str> {
        self.names.get(index as usize)?.as_deref()
    }

    /// Returns how many indices have a name.
    pub fn len(&self) -> usize {
        self.names.iter().filter(|slot| slot.is_some()).count()
    }

    /// Returns true when nothing was declared.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns every declared structure as an index and name pair.
    pub fn iter(&self) -> impl Iterator<Item = (u8, &str)> {
        self.names
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_deref().map(|name| (index as u8, name)))
    }
}

/// Which file and line one line of expanded source came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineOrigin {
    /// Index into [`SourceMap::files`].
    pub file: u32,
    /// 1-based line number within that file.
    pub line: u32,
}

/// Maps expanded source back to the files it was assembled from.
///
/// Without this every diagnostic in an included file would point at a line
/// number in a buffer nobody can open.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<PathBuf>,
    lines: Vec<LineOrigin>,
}

impl SourceMap {
    /// Returns every file that contributed, in the order it first appeared.
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Returns the file at `index`.
    pub fn file(&self, index: u32) -> Option<&Path> {
        self.files.get(index as usize).map(PathBuf::as_path)
    }

    /// Returns where a 1-based line of expanded source came from.
    pub fn origin(&self, expanded_line: u32) -> Option<LineOrigin> {
        let index = (expanded_line as usize).checked_sub(1)?;
        self.lines.get(index).copied()
    }

    /// Returns the file and line a position in the expanded source came from.
    pub fn locate(&self, position: Position) -> Option<(&Path, u32)> {
        let origin = self.origin(position.line)?;
        Some((self.file(origin.file)?, origin.line))
    }

    /// Returns how many lines the expanded source holds.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn intern(&mut self, path: &Path) -> u32 {
        if let Some(index) = self.files.iter().position(|known| known == path) {
            return index as u32;
        }
        self.files.push(path.to_path_buf());
        (self.files.len() - 1) as u32
    }

    fn push(&mut self, origin: LineOrigin) {
        self.lines.push(origin);
    }
}

/// Where the preprocessor looks for included files.
///
/// The reference compiler has no search path at all: an include resolves
/// against the directory of the file that named it. These paths are a fallback
/// searched after that, so a caller can point at a shared script folder without
/// copying files around. First match wins.
#[derive(Debug, Clone, Default)]
pub struct IncludeResolver {
    search_paths: Vec<PathBuf>,
}

impl IncludeResolver {
    /// Builds a resolver with no search paths.
    pub fn new() -> Self {
        IncludeResolver::default()
    }

    /// Adds a directory to the end of the search path.
    pub fn push_path(&mut self, directory: impl Into<PathBuf>) {
        self.search_paths.push(directory.into());
    }

    /// Returns the search path in order.
    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search_paths
    }

    /// Finds `name` on the search path, adding the `.nss` extension if absent.
    ///
    /// Returns `None` when no directory holds it.
    pub fn resolve(&self, name: &str) -> Option<PathBuf> {
        self.resolve_from(None, name)
    }

    /// Finds `name`, looking in `directory` first and the search path after.
    ///
    /// `name` is the raw text between the quotes. Its extension is stripped and
    /// `.nss` is appended, backslashes are treated as separators, and a
    /// case-insensitive match is accepted when an exact one fails. That last
    /// part is a deliberate divergence: the reference compiler runs on a
    /// case-insensitive filesystem, and mod source relies on it.
    pub fn resolve_from(&self, directory: Option<&Path>, name: &str) -> Option<PathBuf> {
        let relative = include_relative_path(name);
        if relative.as_os_str().is_empty() {
            return None;
        }

        directory
            .into_iter()
            .chain(self.search_paths.iter().map(PathBuf::as_path))
            .find_map(|base| find_file(base, &relative))
    }
}

/// One expanded translation unit plus the map back to the files it came from.
#[derive(Debug, Clone, Default)]
pub struct Expansion {
    /// The expanded source text, ready for the lexer.
    pub source: String,
    /// Where each line of [`Expansion::source`] came from.
    pub map: SourceMap,
    /// Every file that contributed, in the order it was pulled in.
    pub included_files: Vec<PathBuf>,
    /// The engine structure types the `#define` lines declared.
    pub engine_structures: EngineStructures,
    /// Warnings raised while expanding. Errors come back as [`CompileError`].
    pub warnings: Vec<Diagnostic>,
}

impl Expansion {
    /// Rewrites a diagnostic to name the file and line it really came from.
    ///
    /// Diagnostics raised by the lexer and the parser carry positions in the
    /// expanded buffer. This maps them back so the message reads
    /// `k_inc_utility.nss(84): Error: ...` rather than pointing at a line of a
    /// buffer that exists only in memory.
    pub fn localize(&self, mut diagnostic: Diagnostic) -> Diagnostic {
        if let Some((path, line)) = self.map.locate(diagnostic.span.start) {
            diagnostic.file = file_label(path);
            let shift = line as i64 - diagnostic.span.start.line as i64;
            diagnostic.span.start.line = line;
            diagnostic.span.end.line = (diagnostic.span.end.line as i64 + shift).max(1) as u32;
        }
        diagnostic
    }

    /// Rewrites a batch of diagnostics with [`Expansion::localize`].
    pub fn localize_all(&self, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        diagnostics
            .into_iter()
            .map(|diagnostic| self.localize(diagnostic))
            .collect()
    }
}

/// Expands `source` into a single translation unit.
///
/// `origin` names the file the text came from. It is used for diagnostics and
/// as the first place to look for a relative include.
///
/// # Errors
///
/// Returns the diagnostics that stopped expansion: an include that could not be
/// opened, a malformed `#define`, or an include tree deeper than
/// [`MAX_INCLUDE_DEPTH`].
pub fn expand(
    source: &str,
    origin: &Path,
    resolver: &IncludeResolver,
) -> Result<Expansion, CompileError> {
    let mut expander = Expander {
        resolver,
        expansion: Expansion::default(),
        diagnostics: Vec::new(),
        errors: 0,
        seen: HashSet::new(),
    };

    expander.seen.insert(dedup_key(origin));
    expander.process(source, origin, 0);

    if expander.errors > 0 {
        return Err(CompileError::Diagnostics(expander.diagnostics));
    }

    expander.expansion.warnings = expander.diagnostics;
    Ok(expander.expansion)
}

struct Expander<'a> {
    resolver: &'a IncludeResolver,
    expansion: Expansion,
    diagnostics: Vec<Diagnostic>,
    errors: usize,
    seen: HashSet<String>,
}

impl Expander<'_> {
    fn process(&mut self, text: &str, path: &Path, depth: usize) {
        let file_index = self.expansion.map.intern(path);
        let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut in_comment = false;

        for (offset, line) in text.split('\n').enumerate() {
            let line_number = offset as u32 + 1;
            let origin = LineOrigin {
                file: file_index,
                line: line_number,
            };

            let directive = if in_comment {
                None
            } else {
                Directive::parse(line)
            };

            match directive {
                None => {
                    self.push_line(line, origin);
                    in_comment = scan_comment_state(line, in_comment);
                }
                Some(Directive::Define(body)) => {
                    self.push_line("", origin);
                    self.handle_define(body, path, line_number);
                }
                Some(Directive::Include(name)) => {
                    self.push_line("", origin);
                    if !self.handle_include(name, &directory, path, line_number, depth) {
                        return;
                    }
                }
            }
        }
    }

    fn push_line(&mut self, line: &str, origin: LineOrigin) {
        self.expansion.source.push_str(line);
        self.expansion.source.push('\n');
        self.expansion.map.push(origin);
    }

    fn handle_define(&mut self, body: &str, path: &Path, line: u32) {
        let mut words = body
            .split(|ch: char| !(0x21..=0x7e).contains(&(ch as u32)))
            .filter(|word| !word.is_empty());

        let (Some(name), Some(value)) = (words.next(), words.next()) else {
            self.error(path, line, "#define syntax");
            return;
        };
        if words.next().is_some() {
            self.error(path, line, "#define syntax");
            return;
        }

        if name == "ENGINE_NUM_STRUCTURES" {
            return;
        }

        if let Some(digits) = name.strip_prefix("ENGINE_STRUCTURE_") {
            // The reference compiler runs this through atol, which takes the
            // leading digits and yields 0 when there are none.
            let index = leading_number(digits);
            match u8::try_from(index) {
                Ok(index) if self.expansion.engine_structures.define(index, value) => {}
                _ => self.error(
                    path,
                    line,
                    format!("Too many defined structures, limit of {MAX_ENGINE_STRUCTURES}"),
                ),
            }
            return;
        }

        self.error(path, line, "#define unknown or invalid definition");
    }

    /// Returns false when expansion must stop.
    fn handle_include(
        &mut self,
        name: &str,
        directory: &Path,
        path: &Path,
        line: u32,
        depth: usize,
    ) -> bool {
        let stripped = strip_include_extension(name);

        if depth >= MAX_INCLUDE_DEPTH {
            self.error(
                path,
                line,
                format!("Unable to open the include file \"{stripped}\": nesting is too deep"),
            );
            return false;
        }

        let Some(resolved) = self.resolver.resolve_from(Some(directory), name) else {
            self.error(
                path,
                line,
                format!("Unable to open the include file \"{stripped}\""),
            );
            return false;
        };

        if !self.seen.insert(dedup_key(&resolved)) {
            return true;
        }

        let bytes = match std::fs::read(&resolved) {
            Ok(bytes) => bytes,
            Err(_) => {
                self.error(
                    path,
                    line,
                    format!("Unable to open the include file \"{stripped}\""),
                );
                return false;
            }
        };

        let text = decode_source(&bytes);
        self.expansion.included_files.push(resolved.clone());
        self.process(&text, &resolved, depth + 1);
        true
    }

    fn error(&mut self, path: &Path, line: u32, message: impl Into<String>) {
        self.errors += 1;
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            stage: Stage::Preprocessor,
            span: Span::point(Position::new(0, line, 1)),
            file: file_label(path),
            message: message.into(),
        });
    }
}

enum Directive<'a> {
    Include(&'a str),
    Define(&'a str),
}

impl<'a> Directive<'a> {
    /// Recognises a directive the way the reference compiler does.
    ///
    /// Both tests are prefix tests, so `#includes "a"` really is an include.
    /// That is faithful, not a bug in this port.
    fn parse(line: &'a str) -> Option<Directive<'a>> {
        let start = line
            .bytes()
            .position(|byte| (0x21..=0x7e).contains(&byte))?;
        let rest = &line[start..];
        if !rest.starts_with('#') {
            return None;
        }

        if let Some(tail) = rest.strip_prefix("#include") {
            return Some(Directive::Include(include_name(tail)));
        }
        if let Some(tail) = rest.strip_prefix("#define") {
            return Some(Directive::Define(tail));
        }
        None
    }
}

/// Pulls the quoted name out of the text after `#include`.
///
/// Scanning stops at the end of the line, so a missing closing quote takes the
/// rest of the line and a missing opening quote yields nothing.
fn include_name(tail: &str) -> &str {
    let Some(open) = tail.find('"') else {
        return "";
    };
    let body = &tail[open + 1..];
    match body.find('"') {
        Some(close) => &body[..close],
        None => body,
    }
}

/// Drops the extension from an include name.
///
/// The last `.` counts, and only when it comes after the last path separator,
/// so `incs\v1.2\inc_a` keeps its directory intact.
fn strip_include_extension(name: &str) -> &str {
    let after_separator = name.rfind(['\\', '/']).map(|index| index + 1).unwrap_or(0);
    match name[after_separator..].rfind('.') {
        Some(dot) => &name[..after_separator + dot],
        None => name,
    }
}

/// Turns an include name into a relative path with a `.nss` extension.
fn include_relative_path(name: &str) -> PathBuf {
    let stripped = strip_include_extension(name);
    let mut path = PathBuf::new();
    for segment in stripped.split(['\\', '/']).filter(|part| !part.is_empty()) {
        path.push(segment);
    }
    if path.as_os_str().is_empty() {
        return path;
    }
    let file_name = format!(
        "{}.nss",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    path.set_file_name(file_name);
    path
}

/// Looks `relative` up under `base`, exactly first and case-insensitively after.
fn find_file(base: &Path, relative: &Path) -> Option<PathBuf> {
    let exact = base.join(relative);
    if exact.is_file() {
        return Some(exact);
    }

    let mut current = base.to_path_buf();
    for component in relative.iter() {
        let wanted = component.to_string_lossy().to_ascii_lowercase();
        let directory = if current.as_os_str().is_empty() {
            Path::new(".")
        } else {
            current.as_path()
        };
        let entries = std::fs::read_dir(directory).ok()?;
        let matched = entries
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase() == wanted)?;
        current.push(matched.file_name());
    }

    current.is_file().then_some(current)
}

/// Builds the key duplicate suppression compares on.
///
/// The reference compiler compares include names with a case-insensitive
/// compare, so two spellings of one file are one include.
fn dedup_key(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

/// Decodes `.nss` bytes into text the rest of the pipeline can hold.
///
/// KOTOR script files are bytes, not UTF-8: they are Windows-1252 in practice
/// and real files in the wild carry accented characters and stray high bytes.
/// Rejecting them, or replacing them with U+FFFD, would both be wrong — a
/// string literal's bytes go into the compiled script verbatim and its length
/// is counted in bytes.
///
/// So every byte maps to the character of the same value, which is Latin-1 and
/// is a bijection over all 256 values. [`encode_source`] is its exact inverse,
/// and code generation must use it when writing a string literal back out.
pub fn decode_source(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| byte as char).collect()
}

/// Turns text back into the bytes it was decoded from.
///
/// The exact inverse of [`decode_source`]. Characters above U+00FF cannot come
/// from a decoded file; they are replaced with `?` rather than silently
/// widening a string literal.
pub fn encode_source(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| u8::try_from(ch as u32).unwrap_or(b'?'))
        .collect()
}

/// Returns the name a diagnostic should print for a file.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Reads the leading decimal digits of `text`, the way `atol` does.
fn leading_number(text: &str) -> u32 {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// Returns whether a block comment is still open at the end of `line`.
///
/// The reference compiler decides whether to preprocess a line by whether its
/// token fetch loop is inside a block comment, so this has to know the same
/// thing. String literals are tracked because `"/*"` opens no comment, and they
/// cannot span lines.
fn scan_comment_state(line: &str, in_comment: bool) -> bool {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut in_comment = in_comment;

    while index < bytes.len() {
        if in_comment {
            if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                in_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                in_comment = true;
                index += 2;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => return false,
            b'"' => {
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'"' => {
                            index += 1;
                            break;
                        }
                        b'\\' => index += 2,
                        _ => index += 1,
                    }
                }
            }
            _ => index += 1,
        }
    }

    in_comment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_text(source: &str) -> Expansion {
        expand(source, Path::new("k_test.nss"), &IncludeResolver::new())
            .expect("the source has no directives that can fail")
    }

    fn expand_error(source: &str) -> Vec<Diagnostic> {
        match expand(source, Path::new("k_test.nss"), &IncludeResolver::new()) {
            Err(CompileError::Diagnostics(diagnostics)) => diagnostics,
            other => panic!("expected diagnostics, got {other:?}"),
        }
    }

    #[test]
    fn a_fresh_resolver_has_no_search_path() {
        let resolver = IncludeResolver::new();
        assert!(resolver.search_paths().is_empty());
        assert!(resolver.resolve("k_inc_generic").is_none());
    }

    #[test]
    fn search_paths_keep_the_order_they_were_added() {
        let mut resolver = IncludeResolver::new();
        resolver.push_path("first");
        resolver.push_path("second");
        assert_eq!(resolver.search_paths().len(), 2);
        assert!(resolver.search_paths()[0].ends_with("first"));
    }

    #[test]
    fn plain_source_passes_through_line_for_line() {
        let expansion = expand_text("void main()\n{\n}\n");
        assert_eq!(expansion.source, "void main()\n{\n}\n\n");
        let (file, line) = expansion
            .map
            .locate(Position::new(0, 3, 1))
            .expect("line three is mapped");
        assert_eq!(file, Path::new("k_test.nss"));
        assert_eq!(line, 3);
    }

    #[test]
    fn engine_structures_come_from_defines_not_from_a_hard_coded_list() {
        let expansion = expand_text(
            "#define ENGINE_NUM_STRUCTURES 4\n\
             #define ENGINE_STRUCTURE_0 effect\n\
             #define ENGINE_STRUCTURE_3 talent\n",
        );
        let structures = &expansion.engine_structures;
        assert_eq!(structures.lookup("effect"), Some(0));
        assert_eq!(structures.lookup("talent"), Some(3));
        assert_eq!(structures.lookup("itemproperty"), None);
        assert_eq!(structures.name(3), Some("talent"));
        assert_eq!(structures.len(), 2);
    }

    #[test]
    fn a_define_that_is_not_an_engine_structure_is_rejected() {
        let diagnostics = expand_error("#define MY_CONST 42\nvoid main() {}\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "#define unknown or invalid definition"
        );
        assert_eq!(diagnostics[0].span.start.line, 1);
        assert_eq!(diagnostics[0].file, "k_test.nss");
    }

    #[test]
    fn a_define_needs_exactly_two_words() {
        assert_eq!(
            expand_error("#define ENGINE_STRUCTURE_0\n")[0].message,
            "#define syntax"
        );
        assert_eq!(
            expand_error("#define ENGINE_STRUCTURE_0 effect extra\n")[0].message,
            "#define syntax"
        );
    }

    #[test]
    fn an_engine_structure_index_past_the_table_is_rejected() {
        let diagnostics = expand_error("#define ENGINE_STRUCTURE_99 thing\n");
        assert_eq!(
            diagnostics[0].message,
            "Too many defined structures, limit of 16"
        );
    }

    #[test]
    fn a_hash_that_is_not_a_directive_reaches_the_lexer() {
        let expansion = expand_text("void main() { int n = #2DAMEMORY5; }\n");
        assert!(expansion.source.contains("#2DAMEMORY5"));
    }

    #[test]
    fn a_directive_inside_a_block_comment_is_not_a_directive() {
        // The reference compiler stops preprocessing while the token fetch loop
        // is inside a comment, so this is source text, not an error.
        let expansion = expand_text("/*\n#define MY_CONST 42\n*/\nvoid main() {}\n");
        assert!(expansion.source.contains("#define MY_CONST 42"));
    }

    #[test]
    fn a_comment_opener_inside_a_string_opens_no_comment() {
        assert!(!scan_comment_state("string s = \"/*\";", false));
        assert!(scan_comment_state("int n; /* trailing", false));
        assert!(!scan_comment_state("still inside */ int n;", true));
        assert!(!scan_comment_state("// /* not a comment opener", false));
    }

    #[test]
    fn include_names_lose_their_extension_but_keep_their_directory() {
        assert_eq!(strip_include_extension("foo.nss"), "foo");
        assert_eq!(strip_include_extension("foo"), "foo");
        assert_eq!(
            strip_include_extension("incs\\v1.2\\inc_a"),
            "incs\\v1.2\\inc_a"
        );
        assert_eq!(strip_include_extension("incs\\inc_a.nss"), "incs\\inc_a");
    }

    #[test]
    fn backslash_include_paths_become_real_paths() {
        assert_eq!(
            include_relative_path("incs\\inc_a"),
            PathBuf::from("incs").join("inc_a.nss")
        );
        assert_eq!(
            include_relative_path("inc_a.nss"),
            PathBuf::from("inc_a.nss")
        );
    }

    #[test]
    fn an_include_name_is_whatever_sits_between_the_quotes() {
        assert_eq!(include_name(" \"inc_a\""), "inc_a");
        assert_eq!(include_name(" \"inc_a"), "inc_a");
        assert_eq!(include_name(" inc_a"), "");
    }

    #[test]
    fn a_missing_include_is_an_error_that_names_the_resref() {
        let diagnostics = expand_error("#include \"no_such_include\"\nvoid main() {}\n");
        assert_eq!(
            diagnostics[0].message,
            "Unable to open the include file \"no_such_include\""
        );
    }

    #[test]
    fn engine_structure_names_are_registered_once() {
        let mut structures = EngineStructures::new();
        assert!(structures.define(0, "effect"));
        assert!(structures.define(1, "effect"));
        assert_eq!(structures.lookup("effect"), Some(0));
        assert_eq!(structures.name(1), None);
        assert!(!structures.define(200, "toofar"));
    }

    #[test]
    fn atol_style_index_parsing_takes_the_leading_digits() {
        assert_eq!(leading_number("12"), 12);
        assert_eq!(leading_number("3x"), 3);
        assert_eq!(leading_number("x"), 0);
    }
}
