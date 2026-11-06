//! Build inputs and their canonical encoding, shared with the regression tests.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use sha2::{Digest, Sha256};
use syn::parse::Parser;
use syn::visit_mut::{self, VisitMut};

pub const SIM_ROOTS: [&str; 4] = [
    "crates/bota_core/src/lib.rs",
    "crates/bota_proto/src/lib.rs",
    "crates/bota_server/src/game/mod.rs",
    "crates/bota_server/src/engine/mod.rs",
];
const PROTO_SIM_MODULES: &[&str] = &[
    "attrs",
    "entity_ids",
    "ids",
    "game_limits",
    "match_rule",
    "math",
    "order",
    "status",
    "talents",
];
const PROTO_WIRE_MODULES: &[&str] = &[
    "bounded",
    "chat",
    "codec",
    "contract",
    "event",
    "fingerprint",
    "limits",
    "msg",
    "peer",
    "registry",
    "unpack",
    "view",
];
const MAX_FILES: usize = 4_096;
const MAX_DEPTH: usize = 32;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const DATA_BOUNDARY: &str = "crates/bota_core/src/data/mod.rs";

pub struct Identity {
    pub digest: u64,
    pub files: BTreeSet<String>,
    pub modules: BTreeSet<String>,
    pub raw_sources: BTreeSet<String>,
}

struct Input {
    path: PathBuf,
    rust: bool,
    module: bool,
}

struct Source {
    text: String,
    raw: bool,
    includes: Vec<Included>,
    modules: Vec<Module>,
}

struct Included {
    path: String,
    rust: bool,
}

struct Module {
    directory: PathBuf,
    name: String,
}

pub fn identity(root: &Path) -> Result<Identity, String> {
    let mut pending: Vec<_> = SIM_ROOTS
        .iter()
        .map(|path| Input {
            path: PathBuf::from(path),
            rust: true,
            module: true,
        })
        .collect();
    let mut inputs = BTreeMap::new();
    let mut files = BTreeSet::new();
    let mut modules = BTreeSet::new();
    let mut total = 0;
    let mut data_entries = 0;
    validate_data_sources(
        root,
        Path::new("crates/bota_core/src/data"),
        0,
        &mut data_entries,
        &mut files,
        &mut inputs,
        &mut total,
    )?;
    while let Some(input) = pending.pop() {
        let name = portable(&input.path)?;
        if name.starts_with("crates/bota_core/src/data/") {
            return Err(format!(
                "{name}: private item data must enter through ITEM_ROWS"
            ));
        }
        if inputs.contains_key(&(name.clone(), input.rust)) {
            continue;
        }
        if inputs.len() >= MAX_FILES {
            return Err("too many rule input files".into());
        }
        let bytes = read(root, &input.path)?;
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err(format!("{name}: rule inputs exceed 64 MiB"));
        }
        files.insert(name.clone());
        if input.module {
            modules.insert(name.clone());
        }
        let (raw, contents) = if input.rust {
            let text = match String::from_utf8(normalize_crlf(&bytes)) {
                Ok(text) => text,
                Err(_) => {
                    warning(
                        &name,
                        "not UTF-8; hashing raw source so rustc can report the error",
                    );
                    inputs.insert((name, true), (true, normalize_crlf(&bytes)));
                    continue;
                }
            };
            let source = source_file(&name, &text)?;
            for include in source.includes {
                if pending.len() >= MAX_FILES {
                    return Err(format!("{name}: too many rule dependencies"));
                }
                pending.push(Input {
                    path: contained(&input.path, &include.path)?,
                    rust: include.rust,
                    module: false,
                });
            }
            if !input.module && !source.modules.is_empty() {
                return Err(format!(
                    "{name}: use normal modules rather than module declarations inside include!"
                ));
            }
            for module in source.modules {
                if pending.len() >= MAX_FILES {
                    return Err(format!("{name}: too many rule dependencies"));
                }
                pending.push(Input {
                    path: module_path(root, &module)?,
                    rust: true,
                    module: true,
                });
            }
            (source.raw, source.text.into_bytes())
        } else {
            (false, bytes)
        };
        inputs.insert((name, input.rust), (raw, contents));
    }
    let mut hash = Sha256::new();
    let mut raw_sources = BTreeSet::new();
    frame(&mut hash, b"bota-rules/source-v2");
    for ((name, rust), (raw, bytes)) in inputs {
        if rust && raw {
            raw_sources.insert(name.clone());
        }
        if bytes.is_empty() {
            continue;
        }
        frame(&mut hash, name.as_bytes());
        frame(
            &mut hash,
            if !rust {
                b"data"
            } else if raw {
                b"rust-raw"
            } else {
                b"rust-ast"
            },
        );
        frame(&mut hash, &bytes);
    }
    let bytes = hash.finalize();
    Ok(Identity {
        digest: u64::from_le_bytes(bytes[..8].try_into().expect("eight digest bytes")),
        files,
        modules,
        raw_sources,
    })
}

fn normalize_crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
            at += 1;
        }
        out.push(bytes[at]);
        at += 1;
    }
    out
}

fn warning(name: &str, message: &str) {
    println!(
        "cargo:warning={name}: {}",
        message.lines().collect::<Vec<_>>().join(" ")
    );
}

fn data_boundary(file: &syn::File) -> Result<(), String> {
    let mut rows = 0;
    for item in file
        .items
        .iter()
        .filter(|item| !test_only(item_attributes(item)))
    {
        if let syn::Item::Use(item) = item
            && !matches!(item.vis, syn::Visibility::Inherited)
        {
            let expected = matches!(&item.tree, syn::UseTree::Path(path) if path.ident == "item_mechanics" && matches!(&*path.tree, syn::UseTree::Name(name) if name.ident == "ITEM_ROWS"));
            if !expected {
                return Err(format!(
                    "{DATA_BOUNDARY}: private item data may re-export only ITEM_ROWS"
                ));
            }
            rows += 1;
        } else if item_visibility(item)
            .is_some_and(|visibility| !matches!(visibility, syn::Visibility::Inherited))
        {
            return Err(format!(
                "{DATA_BOUNDARY}: private item data may export only ITEM_ROWS"
            ));
        }
    }
    if rows != 1 {
        return Err(format!(
            "{DATA_BOUNDARY}: expected exactly one ITEM_ROWS re-export"
        ));
    }
    Ok(())
}

type Inputs = BTreeMap<(String, bool), (bool, Vec<u8>)>;

fn validate_data_sources(
    root: &Path,
    directory: &Path,
    depth: usize,
    entries_seen: &mut usize,
    files: &mut BTreeSet<String>,
    raw: &mut Inputs,
    total: &mut usize,
) -> Result<(), String> {
    let name = portable(directory)?;
    if depth > MAX_DEPTH {
        return Err(format!(
            "{name}: item data exceeds directory depth {MAX_DEPTH}"
        ));
    }
    let path = input_path(root, directory)?
        .ok_or_else(|| format!("{name}: item-data directory not found"))?;
    let entries = std::fs::read_dir(path).map_err(|error| format!("{name}: {error}"))?;
    for entry in entries {
        *entries_seen += 1;
        if *entries_seen > MAX_FILES || files.len() >= MAX_FILES {
            return Err(format!("{name}: too many item-data files"));
        }
        let entry = entry.map_err(|error| format!("{name}: {error}"))?;
        let relative = directory.join(entry.file_name());
        let name = portable(&relative)?;
        let kind = entry
            .file_type()
            .map_err(|error| format!("{name}: {error}"))?;
        if kind.is_symlink() {
            return Err(format!("{name}: item data must not be symlinked"));
        }
        if kind.is_dir() {
            validate_data_sources(root, &relative, depth + 1, entries_seen, files, raw, total)?;
            continue;
        }
        if relative
            .extension()
            .is_none_or(|extension| extension != "rs")
        {
            continue;
        }
        files.insert(name.clone());
        let bytes = normalize_crlf(&read(root, &relative)?);
        *total += bytes.len();
        if *total > MAX_TOTAL_BYTES {
            return Err(format!("{name}: rule inputs exceed 64 MiB"));
        }
        let mut syntax = match std::str::from_utf8(&bytes)
            .ok()
            .and_then(|source| syn::parse_file(source).ok())
        {
            Some(file) => file,
            None => {
                warning(
                    &name,
                    "cannot parse item data; hashing tagged raw source so rustc can report the error",
                );
                raw.insert((name, true), (true, bytes));
                continue;
            }
        };
        let mut fence = DataFence { error: None };
        fence.visit_file_mut(&mut syntax);
        if let Some(error) = fence.error {
            return Err(format!("{name}: {error}"));
        }
        if name == DATA_BOUNDARY {
            data_boundary(&syntax)?;
        }
    }
    if depth == 0 && !files.contains(DATA_BOUNDARY) {
        return Err(format!("{DATA_BOUNDARY}: item-data fence not found"));
    }
    Ok(())
}

struct DataFence {
    error: Option<String>,
}

impl VisitMut for DataFence {
    fn visit_item_impl_mut(&mut self, _: &mut syn::ItemImpl) {
        self.error = Some("impl blocks are forbidden in private item data".into());
    }
    fn visit_item_extern_crate_mut(&mut self, _: &mut syn::ItemExternCrate) {
        self.error = Some("extern declarations are forbidden in private item data".into());
    }
    fn visit_abi_mut(&mut self, _: &mut syn::Abi) {
        self.error = Some("extern declarations are forbidden in private item data".into());
    }
    fn visit_attribute_mut(&mut self, attribute: &mut syn::Attribute) {
        if path_is(attribute.path(), "macro_export") {
            self.error = Some("macro exports are forbidden in private item data".into());
        }
        visit_mut::visit_attribute_mut(self, attribute);
    }
    fn visit_macro_mut(&mut self, item: &mut syn::Macro) {
        if ![
            "assert",
            "assert_eq",
            "assert_ne",
            "debug_assert",
            "debug_assert_eq",
            "debug_assert_ne",
            "matches",
            "panic",
            "unreachable",
        ]
        .iter()
        .any(|name| path_is(&item.path, name))
        {
            self.error = Some(
                "macro declarations and non-builtin macros are forbidden in private item data"
                    .into(),
            );
            return;
        }
        if let Some(keyword) = escaping_tokens(item.tokens.clone(), 0) {
            self.error = Some(format!("{keyword} inside item-data macros is forbidden"));
        }
    }
}

fn escaping_tokens(tokens: TokenStream, depth: usize) -> Option<String> {
    if depth > 128 {
        return Some("deep macro nesting".into());
    }
    for token in tokens {
        match token {
            TokenTree::Ident(name)
                if ["impl", "extern", "macro_rules", "macro_export"]
                    .iter()
                    .any(|keyword| name == keyword) =>
            {
                return Some(name.to_string());
            }
            TokenTree::Group(group) => {
                if let Some(keyword) = escaping_tokens(group.stream(), depth + 1) {
                    return Some(keyword);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn item_identifiers(root: &Path, modules: &BTreeSet<String>) -> Result<String, String> {
    let mut bindings = Bindings {
        values: BTreeMap::new(),
        count: 0,
        error: None,
    };
    for name in modules
        .iter()
        .filter(|name| name.starts_with("crates/bota_proto/src/"))
    {
        let bytes = read(root, Path::new(name))?;
        let text = match String::from_utf8(normalize_crlf(&bytes)) {
            Ok(text) => text,
            Err(_) => {
                warning(
                    name,
                    "invalid UTF-8 identifiers; deferring the source error to rustc",
                );
                continue;
            }
        };
        match syn::parse_file(&text) {
            Ok(mut file) => bindings.visit_file_mut(&mut file),
            Err(_) => {
                warning(
                    name,
                    "cannot parse item identifiers; deferring the source error to rustc",
                );
                continue;
            }
        }
        if let Some(error) = bindings.error {
            return Err(format!("{name}: {error}"));
        }
    }
    let mut code = String::from("const fn item_identifier_digest(id: u16) -> u64 { match id {\n");
    for (id, names) in bindings.values {
        let mut hash = Sha256::new();
        frame(&mut hash, b"bota-item-identifiers/v1");
        frame(&mut hash, &id.to_le_bytes());
        for name in names {
            frame(&mut hash, name.as_bytes());
        }
        let bytes = hash.finalize();
        let digest = u64::from_le_bytes(bytes[..8].try_into().expect("eight digest bytes"));
        code.push_str(&format!("{id} => {digest:#018x},\n"));
    }
    code.push_str("_ => 0, } }\n");
    Ok(code)
}

struct Bindings {
    values: BTreeMap<u16, BTreeSet<String>>,
    count: usize,
    error: Option<String>,
}

impl VisitMut for Bindings {
    fn visit_file_mut(&mut self, file: &mut syn::File) {
        if !test_only(&file.attrs) {
            visit_mut::visit_file_mut(self, file);
        }
    }
    fn visit_item_mut(&mut self, item: &mut syn::Item) {
        if !test_only(item_attributes(item)) {
            visit_mut::visit_item_mut(self, item);
        }
    }

    fn visit_impl_item_mut(&mut self, item: &mut syn::ImplItem) {
        let attributes: &[syn::Attribute] = match item {
            syn::ImplItem::Const(item) => &item.attrs,
            syn::ImplItem::Fn(item) => &item.attrs,
            syn::ImplItem::Type(item) => &item.attrs,
            syn::ImplItem::Macro(item) => &item.attrs,
            _ => &[],
        };
        if !test_only(attributes) {
            visit_mut::visit_impl_item_mut(self, item);
        }
    }

    fn visit_trait_item_mut(&mut self, item: &mut syn::TraitItem) {
        let attributes: &[syn::Attribute] = match item {
            syn::TraitItem::Const(item) => &item.attrs,
            syn::TraitItem::Fn(item) => &item.attrs,
            syn::TraitItem::Type(item) => &item.attrs,
            syn::TraitItem::Macro(item) => &item.attrs,
            _ => &[],
        };
        if !test_only(attributes) {
            visit_mut::visit_trait_item_mut(self, item);
        }
    }

    fn visit_item_impl_mut(&mut self, item: &mut syn::ItemImpl) {
        if item.trait_.is_none() && item_type(&item.self_ty) {
            for item in &item.items {
                let syn::ImplItem::Const(item) = item else {
                    continue;
                };
                if !item_type(&item.ty) || test_only(&item.attrs) {
                    continue;
                }
                self.count += 1;
                if self.count > MAX_FILES {
                    self.error = Some("too many item identifier bindings".into());
                    return;
                }
                let Some(value) = literal_item_id(&item.expr) else {
                    self.error = Some(format!(
                        "item identifier {} must be a literal ItemId(u16)",
                        item.ident
                    ));
                    return;
                };
                self.values
                    .entry(value)
                    .or_default()
                    .insert(item.ident.to_string());
            }
        }
        visit_mut::visit_item_impl_mut(self, item);
    }
}

fn item_type(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(ty) if path_is(&ty.path, "ItemId") || path_is(&ty.path, "Self"))
}

fn literal_item_id(expression: &syn::Expr) -> Option<u16> {
    let syn::Expr::Call(call) = expression else {
        return None;
    };
    let syn::Expr::Path(path) = &*call.func else {
        return None;
    };
    if !(path_is(&path.path, "ItemId") || path_is(&path.path, "Self")) || call.args.len() != 1 {
        return None;
    }
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(value),
        ..
    }) = &call.args[0]
    else {
        return None;
    };
    value.base10_parse().ok()
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn input_path(root: &Path, relative: &Path) -> Result<Option<PathBuf>, String> {
    let name = portable(relative)?;
    let mut path = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err(format!("{name}: rule input escapes the repository"));
        };
        path.push(part);
        match path.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("{name}: rule inputs must not be symlinks"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{name}: {error}")),
        }
    }
    Ok(Some(path))
}

fn read(root: &Path, relative: &Path) -> Result<Vec<u8>, String> {
    let name = portable(relative)?;
    let path =
        input_path(root, relative)?.ok_or_else(|| format!("{name}: rule input not found"))?;
    let meta = path
        .symlink_metadata()
        .map_err(|error| format!("{name}: {error}"))?;
    if !meta.file_type().is_file() {
        return Err(format!("{name}: rule input is not a regular file"));
    }
    let file = std::fs::File::open(path).map_err(|error| format!("{name}: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| format!("{name}: {error}"))?
        .is_file()
    {
        return Err(format!("{name}: rule input is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{name}: {error}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("{name}: rule input exceeds 8 MiB"));
    }
    Ok(bytes)
}

fn module_path(root: &Path, module: &Module) -> Result<PathBuf, String> {
    if module.directory.components().count() > MAX_DEPTH {
        return Err(format!(
            "{}: module tree exceeds depth {MAX_DEPTH}",
            module.directory.display()
        ));
    }
    let direct = module.directory.join(format!("{}.rs", module.name));
    let nested = module.directory.join(&module.name).join("mod.rs");
    match (input_path(root, &direct)?, input_path(root, &nested)?) {
        (Some(_), None) => Ok(direct),
        (None, Some(_)) => Ok(nested),
        (None, None) => Err(format!(
            "{}: module source not found (or {})",
            portable(&direct)?,
            portable(&nested)?
        )),
        (Some(_), Some(_)) => Err(format!(
            "{}: ambiguous module source (also {})",
            portable(&direct)?,
            portable(&nested)?
        )),
    }
}

fn module_directory(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    match path.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "mod.rs" | "main.rs") => parent.to_path_buf(),
        _ => parent.join(path.file_stem().expect("a source filename")),
    }
}

fn portable(path: &Path) -> Result<String, String> {
    path.components()
        .map(|part| match part {
            Component::Normal(name) => name
                .to_str()
                .filter(|name| !name.contains('\\') && !name.chars().any(char::is_control))
                .ok_or_else(|| {
                    "rule input names must be portable UTF-8 without control characters".to_string()
                }),
            _ => Err("rule input escapes the repository".into()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

fn contained(source: &Path, include: &str) -> Result<PathBuf, String> {
    if include.contains('\\') {
        return Err(format!(
            "{}: rule includes must use portable relative paths",
            source.display()
        ));
    }
    let mut relative = source
        .parent()
        .ok_or_else(|| format!("{}: a source has a parent", source.display()))?
        .to_path_buf();
    for part in Path::new(include).components() {
        match part {
            Component::Normal(name) => relative.push(name),
            Component::CurDir => {}
            Component::ParentDir if relative.pop() => {}
            _ => {
                return Err(format!(
                    "{}: rule include escapes the repository",
                    source.display()
                ));
            }
        }
    }
    Ok(relative)
}

#[cfg(test)]
pub fn validate_wire_view(root: &Path) -> Result<(), String> {
    let name = "crates/bota_proto/src/view.rs";
    let bytes = read(root, Path::new(name))?;
    let source = String::from_utf8(bytes).map_err(|error| format!("{name}: {error}"))?;
    let mut syntax = syn::parse_file(&source).map_err(|error| format!("{name}: {error}"))?;
    struct ViewFence(Option<String>);
    impl VisitMut for ViewFence {
        fn visit_item_mut(&mut self, item: &mut syn::Item) {
            if !test_only(item_attributes(item)) {
                visit_mut::visit_item_mut(self, item);
            }
        }
        fn visit_item_impl_mut(&mut self, item: &mut syn::ItemImpl) {
            let methods: &[&str] = match &*item.self_ty {
                syn::Type::Path(ty) if path_is(&ty.path, "SeenBy") => &["by"],
                syn::Type::Path(ty) if path_is(&ty.path, "WorldView") => {
                    &["game_tick", "day_tick", "unit", "player"]
                }
                _ => {
                    self.0 = Some("manual impl belongs in a simulation helper module".into());
                    return;
                }
            };
            if item.trait_.is_some() || item.items.iter().any(|item| !matches!(item, syn::ImplItem::Fn(function) if methods.contains(&function.sig.ident.to_string().as_str()))) {
                self.0 = Some("wire view impl may contain only its existing client-read helpers".into());
            }
            visit_mut::visit_item_impl_mut(self, item);
        }
    }
    let mut fence = ViewFence(None);
    fence.visit_file_mut(&mut syntax);
    fence
        .0
        .map_or(Ok(()), |error| Err(format!("{name}: {error}")))
}

#[cfg(test)]
pub fn canonical(name: &str, source: &str) -> Result<(String, Vec<String>), String> {
    let source = source_file(name, source)?;
    Ok((
        source.text,
        source
            .includes
            .into_iter()
            .map(|include| include.path)
            .collect(),
    ))
}

fn source_file(name: &str, source: &str) -> Result<Source, String> {
    let source = source.replace("\r\n", "\n");
    let mut file = match syn::parse_file(&source) {
        Ok(file) => file,
        Err(error) => {
            warning(
                name,
                &format!(
                    "cannot canonicalize ({error}); hashing tagged raw source so rustc can report the error"
                ),
            );
            return Ok(Source {
                text: source,
                raw: true,
                includes: Vec::new(),
                modules: Vec::new(),
            });
        }
    };
    if test_only(&file.attrs) {
        return Ok(Source {
            text: String::new(),
            raw: false,
            includes: Vec::new(),
            modules: Vec::new(),
        });
    }
    let mut normalize = Normalize {
        name,
        directory: module_directory(Path::new(name)),
        includes: Vec::new(),
        modules: Vec::new(),
        error: None,
    };
    normalize.visit_file_mut(&mut file);
    if let Some(error) = normalize.error {
        return Err(format!("{name}: {error}"));
    }
    let text = if file.items.is_empty() {
        String::new()
    } else {
        prettyplease::unparse(&file)
    };
    Ok(Source {
        text,
        raw: false,
        includes: normalize.includes,
        modules: normalize.modules,
    })
}

struct Normalize<'a> {
    name: &'a str,
    directory: PathBuf,
    includes: Vec<Included>,
    modules: Vec<Module>,
    error: Option<String>,
}

impl Normalize<'_> {
    fn items(&mut self, items: &mut Vec<syn::Item>) {
        items.retain(|item| self.keep_item(item));
        for item in items.iter_mut() {
            self.visit_item_mut(item);
        }
        items.retain(|item| !empty_item(item) && !matches!(item, syn::Item::Const(item) if item.ident == "_" && !contains_items(&item.expr)));
        // rustfmt sorts consecutive imports, including the names within a use tree.
        let mut from = 0;
        while from < items.len() {
            if !matches!(items[from], syn::Item::Use(_)) {
                from += 1;
                continue;
            }
            let end = from
                + items[from..]
                    .iter()
                    .take_while(|item| matches!(item, syn::Item::Use(_)))
                    .count();
            items[from..end].sort_by_cached_key(|item| item.to_token_stream().to_string());
            from = end;
        }
    }

    fn keep_item(&mut self, item: &syn::Item) -> bool {
        if test_only(item_attributes(item)) {
            return false;
        }
        let module = match item {
            syn::Item::Mod(module) => Some(module.ident.to_string()),
            syn::Item::Use(item) => use_root(&item.tree),
            _ => None,
        };
        let root_module = self.directory == module_directory(Path::new(self.name));
        if root_module
            && self.name == "crates/bota_core/src/lib.rs"
            && let Some(name) = &module
        {
            if name == "data"
                && let syn::Item::Mod(item) = item
            {
                if !matches!(item.vis, syn::Visibility::Inherited) || item.content.is_some() {
                    self.error = Some("item data must remain a private out-of-line module".into());
                }
                if item.attrs.iter().any(path_attribute) {
                    self.error = Some("production #[path] modules are not supported".into());
                }
            }
            if ["data", "fingerprint", "text"].contains(&name.as_str()) {
                return false;
            }
        }
        if root_module
            && self.name == "crates/bota_proto/src/lib.rs"
            && let Some(name) = &module
        {
            if PROTO_WIRE_MODULES.contains(&name.as_str()) {
                return false;
            }
            if matches!(item, syn::Item::Mod(_)) && !PROTO_SIM_MODULES.contains(&name.as_str()) {
                self.error = Some(format!(
                    "classify proto module {name} as a simulation helper or wire plumbing"
                ));
            }
        }
        true
    }
}

impl VisitMut for Normalize<'_> {
    fn visit_attributes_mut(&mut self, attributes: &mut Vec<syn::Attribute>) {
        attributes.retain(|attribute| {
            !attribute.path().is_ident("doc") && !inactive_attribute(attribute)
        });
        for attribute in attributes {
            self.visit_attribute_mut(attribute);
        }
    }

    fn visit_file_mut(&mut self, file: &mut syn::File) {
        self.visit_attributes_mut(&mut file.attrs);
        self.items(&mut file.items);
    }

    fn visit_item_mod_mut(&mut self, item: &mut syn::ItemMod) {
        self.visit_attributes_mut(&mut item.attrs);
        if item.attrs.iter().any(path_attribute) {
            self.error = Some("production #[path] modules are not supported".into());
            return;
        }
        if let Some((_, items)) = &mut item.content {
            let directory = self.directory.join(module_name(&item.ident));
            if directory.components().count() > MAX_DEPTH {
                self.error = Some("module tree exceeds depth 32".into());
                return;
            }
            let parent = std::mem::replace(&mut self.directory, directory);
            self.items(items);
            self.directory = parent;
        } else if self.modules.len() < MAX_FILES {
            self.modules.push(Module {
                directory: self.directory.clone(),
                name: module_name(&item.ident),
            });
        } else {
            self.error = Some("too many module declarations".into());
        }
    }

    fn visit_block_mut(&mut self, block: &mut syn::Block) {
        block.stmts.retain(|statement| !matches!(statement, syn::Stmt::Item(item) if test_only(item_attributes(item))));
        visit_mut::visit_block_mut(self, block);
        block
            .stmts
            .retain(|statement| !matches!(statement, syn::Stmt::Item(item) if empty_item(item)));
    }

    fn visit_item_impl_mut(&mut self, item: &mut syn::ItemImpl) {
        let item_ids = self.name.starts_with("crates/bota_proto/src/")
            && item.trait_.is_none()
            && item_type(&item.self_ty);
        item.items.retain(|item| match item {
            syn::ImplItem::Const(item) => {
                !(item_ids && item_type(&item.ty)) && !test_only(&item.attrs)
            }
            syn::ImplItem::Fn(item) => !test_only(&item.attrs),
            syn::ImplItem::Type(item) => !test_only(&item.attrs),
            syn::ImplItem::Macro(item) => !test_only(&item.attrs),
            _ => true,
        });
        visit_mut::visit_item_impl_mut(self, item);
    }

    fn visit_item_trait_mut(&mut self, item: &mut syn::ItemTrait) {
        item.items.retain(|item| {
            !test_only(match item {
                syn::TraitItem::Const(item) => &item.attrs,
                syn::TraitItem::Fn(item) => &item.attrs,
                syn::TraitItem::Type(item) => &item.attrs,
                syn::TraitItem::Macro(item) => &item.attrs,
                _ => &[],
            })
        });
        visit_mut::visit_item_trait_mut(self, item);
    }

    fn visit_fields_named_mut(&mut self, fields: &mut syn::FieldsNamed) {
        fields.named = fields
            .named
            .iter()
            .filter(|field| !test_only(&field.attrs))
            .cloned()
            .collect();
        visit_mut::visit_fields_named_mut(self, fields);
    }

    fn visit_fields_unnamed_mut(&mut self, fields: &mut syn::FieldsUnnamed) {
        fields.unnamed = fields
            .unnamed
            .iter()
            .filter(|field| !test_only(&field.attrs))
            .cloned()
            .collect();
        visit_mut::visit_fields_unnamed_mut(self, fields);
    }

    fn visit_item_enum_mut(&mut self, item: &mut syn::ItemEnum) {
        item.variants = item
            .variants
            .iter()
            .filter(|variant| !test_only(&variant.attrs))
            .cloned()
            .collect();
        visit_mut::visit_item_enum_mut(self, item);
    }

    fn visit_use_tree_mut(&mut self, tree: &mut syn::UseTree) {
        visit_mut::visit_use_tree_mut(self, tree);
        if let syn::UseTree::Group(group) = tree {
            let mut entries: Vec<_> = group.items.iter().cloned().collect();
            entries.sort_by_cached_key(|item| item.to_token_stream().to_string());
            group.items = entries.into_iter().collect();
        }
    }

    fn visit_macro_mut(&mut self, item: &mut syn::Macro) {
        if generated_include(self.name, item) {
            return;
        }
        // Only declarations are code here. Arbitrary macro arguments and matchers
        // can inspect attributes as data, so erasing them would hide rule changes.
        let normalized = if path_is(&item.path, "macro_rules") {
            macro_definition(item.tokens.clone(), 0)
        } else if declaration_macro(self.name, &item.path) {
            // These declaration DSLs forward Rust field metadata unchanged.
            macro_declarations(item.tokens.clone(), 0)
        } else {
            Ok(item.tokens.clone())
        };
        match normalized {
            Ok(tokens) => item.tokens = tokens,
            Err(error) => self.error = Some(error),
        }
        if ["include", "include_str", "include_bytes"]
            .iter()
            .any(|name| path_is(&item.path, name))
        {
            match syn::parse2::<syn::LitStr>(item.tokens.clone()) {
                Ok(path) if self.includes.len() < MAX_FILES => self.includes.push(Included {
                    path: path.value(),
                    rust: path_is(&item.path, "include"),
                }),
                Ok(_) => self.error = Some("too many rule includes".into()),
                Err(_) => {
                    self.error =
                        Some("rule includes must name a literal repository-relative path".into())
                }
            }
        }
        if let Err(error) = included_files(item.tokens.clone(), 0, &mut self.includes) {
            self.error = Some(error);
        }
    }

    fn visit_path_mut(&mut self, path: &mut syn::Path) {
        if self.name.starts_with("crates/bota_server/src/")
            && path.segments.len() >= 2
            && path.segments[0].ident == "crate"
        {
            let module = path.segments[1].ident.to_string();
            if !["game", "engine", "profile"].contains(&module.as_str()) {
                self.error = Some(format!(
                    "simulation module reaches crate::{module}; keep decision helpers under game or engine"
                ));
            }
        }
        visit_mut::visit_path_mut(self, path);
    }
}

fn path_is(path: &syn::Path, name: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

fn declaration_macro(source: &str, path: &syn::Path) -> bool {
    matches!(source,
        "crates/bota_server/src/game/world.rs" if path_is(path, "world_tables"))
        || matches!(source, "crates/bota_proto/src/match_rule.rs" if path_is(path, "kind_set"))
}

fn module_name(name: &syn::Ident) -> String {
    let name = name.to_string();
    name.strip_prefix("r#").unwrap_or(&name).to_owned()
}

fn use_root(tree: &syn::UseTree) -> Option<String> {
    // Root re-exports in the rule crates name one module, never an alias.
    match tree {
        syn::UseTree::Path(path) => Some(path.ident.to_string()),
        _ => None,
    }
}

fn path_attribute(attribute: &syn::Attribute) -> bool {
    if attribute.path().is_ident("path") {
        return true;
    }
    attribute.path().is_ident("cfg_attr")
        && attribute
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|arguments| {
                arguments
                    .iter()
                    .skip(1)
                    .any(|meta| meta.path().is_ident("path"))
            })
}

fn inactive_attribute(attribute: &syn::Attribute) -> bool {
    attribute.path().is_ident("cfg_attr")
        && attribute
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|arguments| {
                arguments
                    .first()
                    .is_some_and(|condition| cfg_without_tests(condition) == Some(false))
            })
}

fn empty_item(item: &syn::Item) -> bool {
    match item {
        syn::Item::Impl(item) => item.trait_.is_none() && item.items.is_empty() && item.attrs.is_empty(),
        syn::Item::Mod(item) => item.content.as_ref().is_some_and(|(_, items)| items.iter().all(|item| matches!(item, syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Inherited)))),
        _ => false,
    }
}

fn contains_items(expression: &syn::Expr) -> bool {
    struct Items(bool);
    impl VisitMut for Items {
        fn visit_item_mut(&mut self, _: &mut syn::Item) {
            self.0 = true;
        }
        fn visit_macro_mut(&mut self, item: &mut syn::Macro) {
            if ![
                "assert",
                "assert_eq",
                "assert_ne",
                "debug_assert",
                "debug_assert_eq",
                "debug_assert_ne",
                "matches",
                "panic",
                "unreachable",
            ]
            .iter()
            .any(|name| path_is(&item.path, name))
            {
                self.0 = true;
                return;
            }
            match syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                .parse2(item.tokens.clone())
            {
                Ok(mut arguments) => {
                    for argument in &mut arguments {
                        self.visit_expr_mut(argument);
                    }
                }
                Err(_) => self.0 = true,
            }
        }
    }
    let mut items = Items(false);
    items.visit_expr_mut(&mut expression.clone());
    items.0
}

fn item_visibility(item: &syn::Item) -> Option<&syn::Visibility> {
    match item {
        syn::Item::Const(item) => Some(&item.vis),
        syn::Item::Enum(item) => Some(&item.vis),
        syn::Item::ExternCrate(item) => Some(&item.vis),
        syn::Item::Fn(item) => Some(&item.vis),
        syn::Item::Mod(item) => Some(&item.vis),
        syn::Item::Static(item) => Some(&item.vis),
        syn::Item::Struct(item) => Some(&item.vis),
        syn::Item::Trait(item) => Some(&item.vis),
        syn::Item::TraitAlias(item) => Some(&item.vis),
        syn::Item::Type(item) => Some(&item.vis),
        syn::Item::Union(item) => Some(&item.vis),
        syn::Item::Use(item) => Some(&item.vis),
        _ => None,
    }
}

// Hash the include expression itself, never the value it generates from this hash.
fn generated_include(name: &str, item: &syn::Macro) -> bool {
    let file_name = match name {
        "crates/bota_core/src/fingerprint.rs" => "/rules_identity.rs",
        "crates/bota_core/src/catalogue.rs" => "/item_identity.rs",
        _ => return false,
    };
    if !path_is(&item.path, "include") {
        return false;
    }
    let Ok(expression) = syn::parse2::<syn::ExprMacro>(item.tokens.clone()) else {
        return false;
    };
    if !path_is(&expression.mac.path, "concat") {
        return false;
    }
    let Ok(arguments) = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
        .parse2(expression.mac.tokens)
    else {
        return false;
    };
    if arguments.len() != 2 {
        return false;
    }
    let syn::Expr::Macro(environment) = &arguments[0] else {
        return false;
    };
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(file),
        ..
    }) = &arguments[1]
    else {
        return false;
    };
    path_is(&environment.mac.path, "env")
        && file.value() == file_name
        && syn::parse2::<syn::LitStr>(environment.mac.tokens.clone())
            .is_ok_and(|key| key.value() == "OUT_DIR")
}

fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::ExternCrate(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::ForeignMod(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Macro(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::TraitAlias(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || (attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Meta>()
                    .is_ok_and(|meta| cfg_without_tests(&meta) == Some(false)))
    })
}

fn cfg_without_tests(meta: &syn::Meta) -> Option<bool> {
    cfg_value(meta, 0)
}

fn cfg_value(meta: &syn::Meta, depth: usize) -> Option<bool> {
    if depth > 128 {
        return None;
    }
    match meta {
        syn::Meta::Path(path) if path.is_ident("test") => Some(false),
        syn::Meta::List(list) => {
            let children =
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                    .parse2(list.tokens.clone())
                    .ok()?;
            if list.path.is_ident("not") && children.len() == 1 {
                return cfg_value(&children[0], depth + 1).map(|value| !value);
            }
            let values: Vec<_> = children
                .iter()
                .map(|child| cfg_value(child, depth + 1))
                .collect();
            if list.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|value| *value == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if list.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|value| *value == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

// Normalize expansion bodies, retaining literal matcher syntax.
fn macro_definition(tokens: TokenStream, depth: usize) -> Result<TokenStream, String> {
    if depth > 128 {
        return Err("rule macros exceed depth 128".into());
    }
    let tokens: Vec<_> = tokens.into_iter().collect();
    let mut out = TokenStream::new();
    for (at, token) in tokens.iter().enumerate() {
        let expansion = at >= 2
            && matches!(&tokens[at - 2], TokenTree::Punct(p) if p.as_char() == '=')
            && matches!(&tokens[at - 1], TokenTree::Punct(p) if p.as_char() == '>');
        let token = match token {
            TokenTree::Group(group) if expansion => TokenTree::Group(Group::new(
                group.delimiter(),
                macro_declarations(group.stream(), depth + 1)?,
            )),
            token => token.clone(),
        };
        out.extend([token]);
    }
    Ok(out)
}

fn macro_declarations(tokens: TokenStream, depth: usize) -> Result<TokenStream, String> {
    if depth > 128 {
        return Err("rule macros exceed depth 128".into());
    }
    let tokens: Vec<_> = tokens.into_iter().collect();
    let mut out = TokenStream::new();
    let mut at = 0;
    while at < tokens.len() {
        if matches!(&tokens[at], TokenTree::Punct(mark) if mark.as_char() == '#') {
            let bracket = at
                + 1
                + usize::from(
                    matches!(tokens.get(at + 1), Some(TokenTree::Punct(mark)) if mark.as_char() == '!'),
                );
            if let Some(TokenTree::Group(group)) = tokens.get(bracket)
                && group.delimiter() == Delimiter::Bracket
                && let Ok(meta) = syn::parse2::<syn::Meta>(group.stream())
            {
                if meta.path().is_ident("doc") {
                    at = bracket + 1;
                    continue;
                }
                let test = meta.path().is_ident("test")
                    || (meta.path().is_ident("cfg")
                        && matches!(meta, syn::Meta::List(list) if syn::parse2::<syn::Meta>(list.tokens.clone()).is_ok_and(|meta| cfg_without_tests(&meta) == Some(false))));
                if test && let Some(end) = declaration_end(&tokens, bracket + 1) {
                    at = end;
                    continue;
                }
            }
        }
        let argument =
            at >= 1 && matches!(&tokens[at - 1], TokenTree::Punct(p) if p.as_char() == '!');
        let token = match &tokens[at] {
            TokenTree::Group(group) if !argument => TokenTree::Group(Group::new(
                group.delimiter(),
                macro_declarations(group.stream(), depth + 1)?,
            )),
            token => token.clone(),
        };
        out.extend([token]);
        at += 1;
    }
    Ok(out)
}

// Opaque metavariables may stand for whole items. Keep them conservatively;
// guessing their end could erase the next production sibling.
fn declaration_end(tokens: &[TokenTree], mut at: usize) -> Option<usize> {
    while matches!(tokens.get(at), Some(TokenTree::Punct(mark)) if mark.as_char() == '#') {
        if !matches!(tokens.get(at + 1), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket)
        {
            return None;
        }
        at += 2;
    }
    if matches!(tokens.get(at), Some(TokenTree::Ident(name)) if name == "pub") {
        at += 1;
        if matches!(tokens.get(at), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis)
        {
            at += 1;
        }
    }
    while matches!(tokens.get(at), Some(TokenTree::Ident(name)) if name == "unsafe" || name == "async" || name == "default")
    {
        at += 1;
    }
    let Some(TokenTree::Ident(kind)) = tokens.get(at) else {
        return None;
    };
    let kind = kind.to_string();
    if ![
        "fn",
        "impl",
        "mod",
        "struct",
        "enum",
        "union",
        "trait",
        "type",
        "const",
        "static",
        "use",
        "extern",
        "macro_rules",
    ]
    .contains(&kind.as_str())
    {
        return None;
    }
    let constant = kind == "const"
        && !matches!(tokens.get(at + 1), Some(TokenTree::Ident(name)) if name == "fn" || name == "unsafe" || name == "async");
    let semicolon = constant || ["static", "type", "use"].contains(&kind.as_str());
    let mut angles = 0usize;
    for (offset, token) in tokens[at..].iter().enumerate() {
        match token {
            TokenTree::Punct(mark) if mark.as_char() == '<' => angles += 1,
            TokenTree::Punct(mark) if mark.as_char() == '>' => angles = angles.saturating_sub(1),
            TokenTree::Punct(mark) if mark.as_char() == ';' && angles == 0 => {
                return Some(at + offset + 1);
            }
            TokenTree::Group(group)
                if group.delimiter() == Delimiter::Brace && angles == 0 && !semicolon =>
            {
                return Some(at + offset + 1);
            }
            _ => {}
        }
    }
    None
}

fn included_files(
    tokens: TokenStream,
    depth: usize,
    files: &mut Vec<Included>,
) -> Result<(), String> {
    if depth > 128 {
        return Err("rule macros exceed depth 128".into());
    }
    let tokens: Vec<_> = tokens.into_iter().collect();
    for (at, token) in tokens.iter().enumerate() {
        if let TokenTree::Ident(name) = token
            && ["include", "include_str", "include_bytes"]
                .iter()
                .any(|candidate| name == candidate)
            && matches!(tokens.get(at + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!')
            && let Some(TokenTree::Group(group)) = tokens.get(at + 2)
        {
            let path = syn::parse2::<syn::LitStr>(group.stream())
                .map_err(|_| "rule includes must name a literal repository-relative path")?;
            if files.len() >= MAX_FILES {
                return Err("too many rule includes".into());
            }
            files.push(Included {
                path: path.value(),
                rust: name == "include",
            });
        }
        if let TokenTree::Group(group) = token {
            included_files(group.stream(), depth + 1, files)?;
        }
    }
    Ok(())
}
