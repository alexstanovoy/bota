//! Build inputs and their canonical encoding, shared with the regression tests.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use sha2::{Digest, Sha256};
use syn::parse::Parser;
use syn::visit_mut::{self, VisitMut};

pub const SOURCE_ROOTS: [&str; 3] = [
    "crates/bota_core/src",
    "crates/bota_proto/src",
    "crates/bota_server/src",
];
const MAX_FILES: usize = 4_096;
const MAX_DEPTH: usize = 32;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

pub struct Identity {
    pub digest: u64,
    pub data_files: BTreeSet<String>,
}

pub fn identity(root: &Path) -> Result<Identity, String> {
    let mut pending = Vec::new();
    let mut entries = 0;
    for source in SOURCE_ROOTS {
        discover(root, Path::new(source), 0, &mut entries, &mut pending)?;
    }
    let mut inputs = BTreeMap::new();
    let mut data_files = BTreeSet::new();
    let mut total = 0;
    while let Some(path) = pending.pop() {
        let name = portable(&path)?;
        if inputs.contains_key(&name) {
            continue;
        }
        if inputs.len() >= MAX_FILES {
            return Err("too many rule input files".into());
        }
        let bytes = read(root, &path)?;
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err("rule inputs exceed 64 MiB".into());
        }
        let normalized = if path.extension().is_some_and(|ext| ext == "rs") {
            let source =
                String::from_utf8(bytes).map_err(|_| format!("{name}: expected UTF-8 Rust"))?;
            let (text, included) = canonical(&name, &source)?;
            for include in included {
                let included = contained(&path, &include)?;
                data_files.insert(portable(&included)?);
                pending.push(included);
            }
            text.into_bytes()
        } else {
            normalize_data(&name, bytes)?
        };
        inputs.insert(name, normalized);
    }
    let mut hash = Sha256::new();
    frame(
        &mut hash,
        b"bota-rules/source-v1;syn=2.0.119;prettyplease=0.2.37",
    );
    for (name, source) in inputs {
        if source.is_empty() {
            continue;
        }
        frame(&mut hash, name.as_bytes());
        frame(&mut hash, &source);
    }
    let bytes = hash.finalize();
    Ok(Identity {
        digest: u64::from_le_bytes(bytes[..8].try_into().expect("eight digest bytes")),
        data_files,
    })
}

pub fn item_identifiers(root: &Path) -> Result<String, String> {
    let source = read(root, Path::new("crates/bota_proto/src/ids.rs"))?;
    let source = String::from_utf8(source).map_err(|_| "expected UTF-8 item identifiers")?;
    let file = syn::parse_file(&source).map_err(|error| error.to_string())?;
    let mut bindings: BTreeMap<u16, BTreeSet<String>> = BTreeMap::new();
    let mut count = 0;
    for item in file.items {
        let syn::Item::Impl(item) = item else {
            continue;
        };
        if !item_type(&item.self_ty) || test_only(&item.attrs) {
            continue;
        }
        for item in item.items {
            let syn::ImplItem::Const(item) = item else {
                continue;
            };
            if !item_type(&item.ty) || test_only(&item.attrs) {
                continue;
            }
            count += 1;
            if count > MAX_FILES {
                return Err("too many item identifier bindings".into());
            }
            let value = literal_item_id(&item.expr).ok_or_else(|| {
                format!(
                    "item identifier {} must be a literal ItemId(u16)",
                    item.ident
                )
            })?;
            bindings
                .entry(value)
                .or_default()
                .insert(item.ident.to_string());
        }
    }
    let mut code = String::from("const fn item_identifier_digest(id: u16) -> u64 { match id {\n");
    for (id, names) in bindings {
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

fn item_type(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(ty) if ty.path.is_ident("ItemId") || ty.path.is_ident("Self"))
}

fn literal_item_id(expression: &syn::Expr) -> Option<u16> {
    let syn::Expr::Call(call) = expression else {
        return None;
    };
    let syn::Expr::Path(path) = &*call.func else {
        return None;
    };
    if !(path.path.is_ident("ItemId") || path.path.is_ident("Self")) || call.args.len() != 1 {
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

fn discover(
    root: &Path,
    relative: &Path,
    depth: usize,
    entries: &mut usize,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("rule source directories exceed depth 32".into());
    }
    let directory = checked(root, relative)?;
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        *entries += 1;
        if *entries > MAX_FILES {
            return Err("too many entries in rule source directories".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("rule input names must be UTF-8")?;
        if name == "tests" || name == "tests.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        let path = relative.join(name);
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "{}: rule inputs must not be symlinks",
                portable(&path)?
            ));
        }
        if kind.is_dir() {
            discover(root, &path, depth + 1, entries, files)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            if files.len() >= MAX_FILES {
                return Err("too many rule source files".into());
            }
            files.push(path);
        }
    }
    Ok(())
}

fn checked(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err("rule input escapes the repository".into());
        };
        path.push(part);
        if path
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("rule inputs must not be symlinks".into());
        }
    }
    Ok(path)
}

fn read(root: &Path, relative: &Path) -> Result<Vec<u8>, String> {
    let path = checked(root, relative)?;
    if !path
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("rule input is not a regular file".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("rule input is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("rule input exceeds 8 MiB".into());
    }
    Ok(bytes)
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
        return Err("rule includes must use portable relative paths".into());
    }
    let mut relative = source
        .parent()
        .ok_or("a source has a parent")?
        .to_path_buf();
    for part in Path::new(include).components() {
        match part {
            Component::Normal(name) => relative.push(name),
            Component::CurDir => {}
            Component::ParentDir if relative.pop() => {}
            _ => return Err("rule include escapes the repository".into()),
        }
    }
    Ok(relative)
}

fn normalize_data(name: &str, bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if name.starts_with("crates/bota_proto/registry/") {
        let text = String::from_utf8(bytes).map_err(|_| "expected a UTF-8 registry")?;
        let lines: Vec<_> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|line| {
                name != "crates/bota_proto/registry/ids.txt" || !line.starts_with("item ")
            })
            .collect();
        return Ok(lines.join("\n").into_bytes());
    }
    Ok(bytes)
}

pub fn canonical(name: &str, source: &str) -> Result<(String, Vec<String>), String> {
    let mut file =
        syn::parse_file(&source.replace("\r\n", "\n")).map_err(|e| format!("{name}: {e}"))?;
    if test_only(&file.attrs) {
        return Ok((String::new(), Vec::new()));
    }
    let mut normalize = Normalize {
        name,
        includes: Vec::new(),
        error: None,
    };
    normalize.visit_file_mut(&mut file);
    if let Some(error) = normalize.error {
        return Err(format!("{name}: {error}"));
    }
    if file.items.is_empty() {
        return Ok((String::new(), normalize.includes));
    }
    Ok((prettyplease::unparse(&file), normalize.includes))
}

struct Normalize<'a> {
    name: &'a str,
    includes: Vec<String>,
    error: Option<String>,
}

impl Normalize<'_> {
    fn items(&mut self, items: &mut Vec<syn::Item>) {
        items.retain(|item| match item {
            syn::Item::Const(item) if item.ident == "_" => false,
            syn::Item::Const(item) if self.name == "crates/bota_core/src/data/items.rs" => {
                // The catalogue hashes evaluated item rows, including every imported number.
                !matches!(&*item.ty, syn::Type::Path(ty) if ty.path.is_ident("ItemData"))
            }
            syn::Item::Const(item)
                if self.name == "crates/bota_core/src/data/item_mechanics.rs"
                    && item.ident == "ITEM_ROWS" =>
            {
                false
            }
            _ => !test_only(item_attributes(item)),
        });
        for item in items.iter_mut() {
            self.visit_item_mut(item);
        }
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
}

impl VisitMut for Normalize<'_> {
    fn visit_attributes_mut(&mut self, attributes: &mut Vec<syn::Attribute>) {
        attributes.retain(|attribute| !attribute.path().is_ident("doc"));
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
        if let Some((_, items)) = &mut item.content {
            self.items(items);
        }
    }

    fn visit_block_mut(&mut self, block: &mut syn::Block) {
        block.stmts.retain(|statement| !matches!(statement, syn::Stmt::Item(item) if test_only(item_attributes(item))));
        visit_mut::visit_block_mut(self, block);
    }

    fn visit_item_impl_mut(&mut self, item: &mut syn::ItemImpl) {
        let item_ids = self.name == "crates/bota_proto/src/ids.rs"
            && matches!(&*item.self_ty, syn::Type::Path(ty) if ty.path.is_ident("ItemId"));
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
        let normalized = if item.path.is_ident("macro_rules") {
            macro_definition(item.tokens.clone(), 0)
        } else if item.path.is_ident("world_tables") {
            // This declaration DSL forwards Rust field metadata unchanged.
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
            .any(|name| item.path.is_ident(name))
        {
            match syn::parse2::<syn::LitStr>(item.tokens.clone()) {
                Ok(path) => self.includes.push(path.value()),
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
}

// Hash the include expression itself, never the value it generates from this hash.
fn generated_include(name: &str, item: &syn::Macro) -> bool {
    let file_name = match name {
        "crates/bota_core/src/fingerprint.rs" => "/rules_identity.rs",
        "crates/bota_core/src/catalogue.rs" => "/item_identity.rs",
        _ => return false,
    };
    if !item.path.is_ident("include") {
        return false;
    }
    let Ok(expression) = syn::parse2::<syn::ExprMacro>(item.tokens.clone()) else {
        return false;
    };
    if !expression.mac.path.is_ident("concat") {
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
    environment.mac.path.is_ident("env")
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
                if test {
                    at = bracket + 1;
                    while at < tokens.len() {
                        let end = matches!(&tokens[at], TokenTree::Group(group) if group.delimiter() == Delimiter::Brace)
                            || matches!(&tokens[at], TokenTree::Punct(mark) if mark.as_char() == ';');
                        at += 1;
                        if end {
                            break;
                        }
                    }
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

fn included_files(
    tokens: TokenStream,
    depth: usize,
    files: &mut Vec<String>,
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
            files.push(path.value());
        }
        if let TokenTree::Group(group) = token {
            included_files(group.stream(), depth + 1, files)?;
        }
    }
    Ok(())
}
