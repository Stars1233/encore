use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::rc::Rc;

use anyhow::Result;
use serde::Serialize;
use swc_common::errors::HANDLER;
use swc_common::sync::Lrc;
use swc_common::Spanned;
use swc_ecma_ast as ast;

use crate::parser::module_loader::ModuleId;
use crate::parser::types::ast_id::AstId;
use crate::parser::types::binding::bindings;
use crate::parser::types::typ;
use crate::parser::{module_loader, Range};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct ObjectId(pub(super) usize);

/// An Object describes a named language entity such as a module, constant, type, variable, function, etc.
pub struct Object {
    pub id: ObjectId,
    pub range: Range,
    pub name: Option<String>,
    pub kind: ObjectKind,
    pub module_id: ModuleId,
    pub(super) state: RefCell<CheckState>,
}

impl Serialize for Object {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name.as_ref().unwrap_or(&"".to_string()))
    }
}

impl Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Object")
            // .field("id", &self.id)
            // .field("range", &self.range)
            .field("name", &self.name)
            // .field("kind", &self.kind)
            // .field("module_id", &self.module_id)
            .finish()
    }
}

impl Hash for Object {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Object {}

#[derive(Debug)]
pub enum ObjectKind {
    TypeName(TypeName),
    Enum(Enum),
    Var(Var),
    Using(Using),
    Func(Func),
    Class(Class),
    Module(Rc<Module>),
    Namespace(Namespace),
}

impl ObjectKind {
    pub fn type_params<'a>(&'a self) -> Box<dyn Iterator<Item = &'a ast::TsTypeParam> + 'a> {
        match self {
            ObjectKind::TypeName(TypeName { decl }) => match decl {
                TypeNameDecl::Interface(i) => {
                    Box::new(i.type_params.iter().flat_map(|p| p.params.iter()))
                }
                TypeNameDecl::TypeAlias(t) => {
                    Box::new(t.type_params.iter().flat_map(|p| p.params.iter()))
                }
            },
            _ => Box::new([].iter()),
        }
    }
}

#[derive(Debug)]
pub(super) enum CheckState {
    NotStarted,
    InProgress,
    Completed(typ::Type),
}

#[derive(Debug)]
pub struct TypeName {
    pub decl: TypeNameDecl,
}

#[derive(Debug)]
pub enum TypeNameDecl {
    Interface(ast::TsInterfaceDecl),
    TypeAlias(ast::TsTypeAliasDecl),
}

#[derive(Debug)]
pub struct Class {
    #[allow(dead_code)]
    pub spec: Box<ast::Class>,
}

#[derive(Debug)]
pub struct Enum {
    pub members: Vec<ast::TsEnumMember>,
}

#[derive(Debug)]
pub struct Func {
    #[allow(dead_code)]
    pub spec: Box<ast::Function>,
}

#[derive(Debug)]
pub struct Namespace {
    pub data: Box<NSData>,
}

#[derive(Debug)]
pub struct Var {
    pub type_ann: Option<ast::TsTypeAnn>,
    pub expr: Option<Box<ast::Expr>>,
}

#[derive(Debug)]
pub struct Using {
    pub type_ann: Option<ast::TsTypeAnn>,
    pub expr: Option<Box<ast::Expr>>,
}

#[derive(Debug)]
pub struct NamedReexport {
    pub orig_name: String,
    pub renamed: Option<String>,
}

#[derive(Debug)]
pub enum Reexport {
    List {
        items: Vec<NamedReexport>,
        import_path: String,
    },
    All {
        import_path: String,
    },
}

#[derive(Debug)]
pub struct NSData {
    /// The objects imported by the module.
    pub imports: HashMap<AstId, ImportedName>,

    /// Top-level objects, keyed by their id.
    pub top_level: HashMap<AstId, Rc<Object>>,

    /// The named exports.
    pub named_exports: HashMap<String, Rc<Object>>,

    /// The default export, if any.
    pub default_export: Option<Rc<Object>>,

    // Reexports from other modules.
    pub reexports: Vec<Reexport>,

    /// Export items that haven't yet been processed.
    #[allow(dead_code)]
    pub unprocessed_exports: Vec<ast::ModuleItem>,
}

#[derive(Debug)]
pub struct Module {
    pub base: Lrc<module_loader::Module>,
    pub data: Box<NSData>,
}

#[derive(Debug, Clone)]
pub struct ImportedName {
    pub range: Range,
    pub import_path: String,
    pub kind: ImportKind,
}

#[derive(Debug, Clone)]
pub enum ImportKind {
    Named(String),
    Default,
    Namespace,
}

impl NSData {
    fn new() -> Self {
        Self {
            imports: HashMap::new(),
            top_level: HashMap::new(),
            named_exports: HashMap::new(),
            default_export: None,
            reexports: vec![],
            unprocessed_exports: vec![],
        }
    }

    pub fn get_named_export(
        &self,
        ctx: &ResolveState,
        curr_module: &swc_common::FileName,
        needle: &str,
    ) -> Option<Rc<Object>> {
        if needle == "default" {
            if let Some(default) = &self.default_export {
                return Some(default.clone());
            }
        }
        if let Some(obj) = self.named_exports.get(needle) {
            return Some(obj.clone());
        }

        for re in &self.reexports {
            match re {
                Reexport::List { import_path, items } => {
                    for item in items {
                        let export_name = item.renamed.as_ref().unwrap_or(&item.orig_name);
                        if export_name == needle {
                            let module = ctx.resolve_module_import(curr_module, import_path)?;
                            return module.data.get_named_export(
                                ctx,
                                &module.base.swc_file_path,
                                &item.orig_name,
                            );
                        }
                    }
                }

                Reexport::All { import_path } => {
                    if let Some(module) = ctx.resolve_module_import(curr_module, import_path) {
                        if let Some(export) = module.data.get_named_export(ctx, curr_module, needle)
                        {
                            return Some(export);
                        }
                    }
                }
            }
        }
        None
    }

    fn add_top_level(&mut self, id: AstId, obj: Rc<Object>) -> Rc<Object> {
        if let Some(other) = self.top_level.get(&id) {
            // Unhandled overload most likely, return the existing object for now.
            return other.clone();
        }

        self.top_level.insert(id, obj.clone());
        obj
    }

    fn add_import(&mut self, id: AstId, import: ImportedName) {
        if self.imports.contains_key(&id) {
            HANDLER.with(|handler| {
                handler.span_err(import.range.to_span(), &format!("`{id}` already imported"));
            });
            return;
        }

        self.imports.insert(id, import);
    }
}

fn process_module_items(ctx: &ResolveState, ns: &mut NSData, items: &[ast::ModuleItem]) {
    for it in items {
        match it {
            ast::ModuleItem::ModuleDecl(md) => match md {
                ast::ModuleDecl::Import(import) => process_import(ns, import),
                ast::ModuleDecl::ExportDecl(decl) => {
                    let objs = process_decl(ctx, ns, &decl.decl);
                    for obj in objs {
                        if let Some(name) = &obj.name {
                            ns.named_exports.insert(name.clone(), obj);
                        }
                    }
                }

                ast::ModuleDecl::ExportDefaultDecl(decl) => {
                    let obj = process_default_decl(ctx, &decl.decl);
                    if ns.default_export.is_some() {
                        obj.range.err("duplicate default export");
                    }
                    ns.default_export = Some(obj);
                }

                // TODO(andre) Can this affect the module namespace?
                ast::ModuleDecl::ExportDefaultExpr(_expr) => {
                    // TODO this is e.g `export default new SQLDatabase`
                    // need to resolve to object
                    log::debug!("TODO export default expr");
                }

                ast::ModuleDecl::ExportNamed(decl) => {
                    // Re-exporting from another module.
                    let Some(src) = &decl.src else {
                        log::debug!("ExportNamed without src");
                        continue;
                    };

                    ns.reexports.push(Reexport::List {
                        import_path: src.value.to_string(),
                        items: decl
                            .specifiers
                            .iter()
                            .filter_map(|spec| match spec {
                                ast::ExportSpecifier::Named(named) => {
                                    let orig_name = module_export_name_to_string(&named.orig);
                                    Some(NamedReexport {
                                        orig_name,
                                        renamed: named
                                            .exported
                                            .as_ref()
                                            .map(module_export_name_to_string),
                                    })
                                }
                                ast::ExportSpecifier::Default(_) => {
                                    log::debug!("TODO: ExportNamed with default");
                                    None
                                }
                                ast::ExportSpecifier::Namespace(_) => {
                                    log::debug!("TODO: ExportNamed with namespace");
                                    None
                                }
                            })
                            .collect(),
                    });
                }

                ast::ModuleDecl::ExportAll(decl) => {
                    // Re-exporting * from another module.
                    ns.reexports.push(Reexport::All {
                        import_path: decl.src.value.to_string(),
                    });
                }

                ast::ModuleDecl::TsImportEquals(_) => {
                    log::debug!("TODO ts import equals");
                }

                ast::ModuleDecl::TsExportAssignment(decl) => {
                    log::debug!("TsExportAssignment {:#?}", decl);
                }

                ast::ModuleDecl::TsNamespaceExport(decl) => {
                    log::debug!("TsNamespaceExport {:#?}", decl);
                }
            },

            ast::ModuleItem::Stmt(stmt) => {
                process_stmt(ctx, ns, stmt);
            }
        }
    }
}

/// Process an import declaration, adding imports to the module.
fn process_import(ns: &mut NSData, import: &ast::ImportDecl) {
    for specifier in &import.specifiers {
        match specifier {
            ast::ImportSpecifier::Named(named) => {
                let export_name = named.imported.as_ref().map_or_else(
                    || named.local.clone(),
                    |export_name| match export_name {
                        ast::ModuleExportName::Ident(id) => id.clone(),
                        ast::ModuleExportName::Str(str) => {
                            ast::Ident::new(str.value.clone(), str.span)
                        }
                    },
                );

                ns.add_import(
                    AstId::from(&named.local),
                    ImportedName {
                        range: named.span.into(),
                        import_path: import.src.value.to_string(),
                        kind: ImportKind::Named(export_name.sym.as_ref().to_string()),
                    },
                );
            }
            ast::ImportSpecifier::Default(default) => {
                ns.add_import(
                    AstId::from(&default.local),
                    ImportedName {
                        range: default.span.into(),
                        import_path: import.src.value.to_string(),
                        kind: ImportKind::Default,
                    },
                );
            }
            ast::ImportSpecifier::Namespace(ns_import) => {
                // import * as foo
                ns.add_import(
                    AstId::from(&ns_import.local),
                    ImportedName {
                        range: ns_import.span.into(),
                        import_path: import.src.value.to_string(),
                        kind: ImportKind::Namespace,
                    },
                );
            }
        }
    }
}

fn process_stmt(ctx: &ResolveState, ns: &mut NSData, stmt: &ast::Stmt) -> Vec<Rc<Object>> {
    match stmt {
        ast::Stmt::Decl(decl) => process_decl(ctx, ns, decl),
        ast::Stmt::Block(block) => {
            let mut objs = vec![];
            for stmt in &block.stmts {
                objs.extend(process_stmt(ctx, ns, stmt));
            }
            objs
        }

        // NOTE(andre): I believe other statements can't really declare things,
        // since they're inside blocks.
        _ => vec![],
    }
}

fn process_decl(ctx: &ResolveState, ns: &mut NSData, decl: &ast::Decl) -> Vec<Rc<Object>> {
    let range: Range = decl.span().into();
    match decl {
        ast::Decl::Class(d) => {
            let name = Some(d.ident.sym.to_string());
            let obj = ctx.new_obj(
                name,
                range,
                ObjectKind::Class(Class {
                    spec: d.class.clone(),
                }),
            );
            ns.add_top_level(AstId::from(&d.ident), obj.clone());
            vec![obj]
        }

        ast::Decl::Fn(d) => {
            let name = Some(d.ident.sym.to_string());
            let obj = ctx.new_obj(
                name,
                range,
                ObjectKind::Func(Func {
                    spec: d.function.clone(),
                }),
            );
            ns.add_top_level(AstId::from(&d.ident), obj.clone());
            vec![obj]
        }

        ast::Decl::Var(d) => {
            let mut objs = vec![];
            for var_decl in &d.decls {
                for b in bindings(&var_decl.name) {
                    let name = Some(b.name.to_string());
                    let range = var_decl.span.into();
                    let obj = ctx.new_obj(
                        name,
                        range,
                        ObjectKind::Var(Var {
                            type_ann: b.type_ann,
                            expr: var_decl.init.clone(),
                        }),
                    );
                    ns.add_top_level(AstId::new(b.id, b.name.clone()), obj.clone());
                    objs.push(obj);
                }
            }
            objs
        }

        ast::Decl::Using(d) => {
            let mut objs = vec![];
            for var_decl in &d.decls {
                for b in bindings(&var_decl.name) {
                    let name = Some(b.name.to_string());
                    let range = var_decl.span.into();
                    let obj = ctx.new_obj(
                        name,
                        range,
                        ObjectKind::Using(Using {
                            type_ann: b.type_ann,
                            expr: var_decl.init.clone(),
                        }),
                    );
                    ns.add_top_level(AstId::new(b.id, b.name.clone()), obj.clone());
                    objs.push(obj);
                }
            }
            objs
        }

        ast::Decl::TsInterface(d) => {
            let name = Some(d.id.sym.to_string());
            let obj = ctx.new_obj(
                name,
                range,
                ObjectKind::TypeName(TypeName {
                    decl: TypeNameDecl::Interface(*d.clone()),
                }),
            );
            ns.add_top_level(AstId::from(&d.id), obj.clone());
            vec![obj]
        }

        ast::Decl::TsTypeAlias(d) => {
            let name = d.id.sym.to_string();
            let obj = ctx.new_obj(
                Some(name),
                range,
                ObjectKind::TypeName(TypeName {
                    decl: TypeNameDecl::TypeAlias(*d.clone()),
                }),
            );
            ns.add_top_level(AstId::from(&d.id), obj.clone());
            vec![obj]
        }

        ast::Decl::TsEnum(d) => {
            let name = Some(d.id.sym.to_string());
            let obj = ctx.new_obj(
                name,
                range,
                ObjectKind::Enum(Enum {
                    members: d.members.clone(),
                }),
            );
            ns.add_top_level(AstId::from(&d.id), obj.clone());
            vec![obj]
        }

        ast::Decl::TsModule(d) => {
            // Namespace declaration
            match &d.id {
                ast::TsModuleName::Ident(id) => {
                    let mut ns2 = Namespace {
                        data: Box::new(NSData::new()),
                    };
                    if let Some(body) = &d.body {
                        process_namespace_body(ctx, &mut ns2.data, body);
                    }

                    let name = Some(id.sym.to_string());
                    let obj = ctx.new_obj(name, range, ObjectKind::Namespace(ns2));
                    ns.add_top_level(AstId::from(id), obj.clone());
                    vec![obj]
                }
                ast::TsModuleName::Str(_) => {
                    // This is not valid for namespace declarations, ignore it.
                    vec![]
                }
            }
        }
    }
}

fn process_default_decl(ctx: &ResolveState, decl: &ast::DefaultDecl) -> Rc<Object> {
    let range: Range = decl.span().into();
    match decl {
        ast::DefaultDecl::Class(d) => {
            let name = d.ident.as_ref().map(|id| id.sym.to_string());
            ctx.new_obj(
                name,
                range,
                ObjectKind::Class(Class {
                    spec: d.class.clone(),
                }),
            )
        }

        ast::DefaultDecl::Fn(d) => {
            let name = d.ident.as_ref().map(|id| id.sym.to_string());
            ctx.new_obj(
                name,
                range,
                ObjectKind::Func(Func {
                    spec: d.function.clone(),
                }),
            )
        }

        ast::DefaultDecl::TsInterfaceDecl(d) => {
            let name = Some(d.id.sym.to_string());
            ctx.new_obj(
                name,
                range,
                ObjectKind::TypeName(TypeName {
                    decl: TypeNameDecl::Interface(*d.clone()),
                }),
            )
        }
    }
}

fn process_namespace_body(ctx: &ResolveState, ns: &mut NSData, body: &ast::TsNamespaceBody) {
    match body {
        ast::TsNamespaceBody::TsModuleBlock(block) => {
            process_module_items(ctx, ns, &block.body[..]);
        }
        ast::TsNamespaceBody::TsNamespaceDecl(decl) => {
            let name = Some(decl.id.sym.to_string());
            let mut ns2 = Namespace {
                data: Box::new(NSData::new()),
            };
            process_namespace_body(ctx, &mut ns2.data, &decl.body);

            let range = decl.span.into();
            let obj = ctx.new_obj(name, range, ObjectKind::Namespace(ns2));
            ns.add_top_level(AstId::from(&decl.id), obj);
        }
    }
}

#[derive(Debug)]
pub struct ResolveState {
    loader: Lrc<module_loader::ModuleLoader>,
    module_objects: RefCell<HashMap<ModuleId, Rc<Object>>>,
    module_stack: RefCell<Vec<ModuleId>>,
    universe: OnceCell<Rc<Module>>,
    next_id: Cell<usize>,
}

impl ResolveState {
    pub(super) fn new(loader: Lrc<module_loader::ModuleLoader>) -> Self {
        Self {
            loader,
            module_objects: RefCell::new(HashMap::new()),
            module_stack: RefCell::new(vec![]),
            universe: OnceCell::new(),
            next_id: Cell::new(1),
        }
    }

    pub(super) fn universe(&self) -> Rc<Module> {
        if let Some(universe) = self.universe.get() {
            return universe.to_owned();
        }

        let ast = self.loader.universe();
        let module = self.get_or_init_module(ast);
        self.universe.set(module.clone()).unwrap();
        self.universe.get().unwrap().to_owned()
    }

    pub(super) fn new_obj(
        &self,
        name: Option<String>,
        range: Range,
        kind: ObjectKind,
    ) -> Rc<Object> {
        let obj_id = self.next_id.get();
        self.next_id.set(obj_id + 1);

        let module_id = self.module_id().expect("no current module");
        Rc::new(Object {
            id: ObjectId(obj_id),
            range,
            module_id,
            name,
            kind,
            state: RefCell::new(CheckState::NotStarted),
        })
    }

    pub fn lookup_module(&self, id: ModuleId) -> Option<Rc<Module>> {
        self.module_objects
            .borrow()
            .get(&id)
            .and_then(|obj| match &obj.kind {
                ObjectKind::Module(module) => Some(module.clone()),
                _ => None,
            })
    }

    pub fn is_universe(&self, id: ModuleId) -> bool {
        let universe = self.universe();
        universe.base.id == id
    }

    pub fn is_module_path(&self, id: ModuleId, name: &str) -> bool {
        if let Some(module) = self.lookup_module(id) {
            module.base.module_path.as_ref().is_some_and(|p| p == name)
        } else {
            false
        }
    }

    pub fn get_or_init_module(&self, module: Lrc<module_loader::Module>) -> Rc<Module> {
        let module_id = module.id;
        if let Some(m) = self.lookup_module(module_id) {
            return m;
        }

        let mut data = Box::new(NSData::new());
        self.with_curr_module(module_id, || {
            process_module_items(self, &mut data, &module.ast.body[..])
        });

        let new_module = Rc::new(Module { base: module, data });

        self.module_objects.borrow_mut().insert(
            module_id,
            self.with_curr_module(module_id, || {
                self.new_obj(
                    None,
                    new_module.base.ast.span.into(),
                    ObjectKind::Module(new_module.clone()),
                )
            }),
        );

        new_module
    }

    fn with_curr_module<Fn, Res>(&self, module_id: ModuleId, f: Fn) -> Res
    where
        Fn: FnOnce() -> Res,
    {
        self.module_stack.borrow_mut().push(module_id);
        let result = f();
        self.module_stack.borrow_mut().pop();
        result
    }

    fn module_id(&self) -> Result<ModuleId> {
        let stack = self.module_stack.borrow();
        let module = stack
            .last()
            .ok_or_else(|| anyhow::anyhow!("internal error: no module on stack"))?;
        Ok(module.to_owned())
    }

    pub(super) fn resolve_module_default_export(&self, module_id: ModuleId) -> Option<Rc<Object>> {
        self.lookup_module(module_id)?.data.default_export.clone()
    }

    pub(super) fn resolve_module_ident(
        &self,
        module_id: ModuleId,
        ident: &ast::Ident,
    ) -> Option<Rc<Object>> {
        let module = self.lookup_module(module_id)?;

        // Is it a top-level object in this module?
        let ast_id = AstId::from(ident);
        if let Some(obj) = module.data.top_level.get(&ast_id) {
            return Some(obj.clone());
        }

        // Otherwise, is it an import?
        if let Some(imp_name) = module.data.imports.get(&ast_id) {
            return self.resolve_import(&module, imp_name);
        }

        // Is it in universe scope?
        {
            let universe = self.universe();
            let name = ident.sym.as_ref();
            if let Some(obj) = universe.data.named_exports.get(name) {
                return Some(obj.clone());
            }
        }

        // Otherwise we don't know about this object.
        None
    }

    pub(super) fn resolve_module_import(
        &self,
        from_file: &swc_common::FileName,
        import_path: &str,
    ) -> Option<Rc<Module>> {
        let ast_module = match self.loader.resolve_import(from_file, import_path) {
            Ok(Some(ast_module)) => ast_module,
            Ok(None) | Err(_) => return None,
        };
        Some(self.get_or_init_module(ast_module))
    }

    pub(super) fn resolve_import(&self, module: &Module, imp: &ImportedName) -> Option<Rc<Object>> {
        let ast_module = match self
            .loader
            .resolve_import(&module.base.swc_file_path, &imp.import_path)
        {
            Ok(None) => return None,
            Ok(Some(ast_module)) => Ok(ast_module),
            Err(err) => Err(err),
        };

        let ast_module = ast_module
            .inspect_err(|err| {
                HANDLER.with(|handler| {
                    handler.span_err(imp.range.to_span(), &format!("import not found: {err}"))
                })
            })
            .ok()?;

        match &imp.kind {
            ImportKind::Named(name) => {
                let imported = self.get_or_init_module(ast_module);
                let obj = imported
                    .data
                    .get_named_export(self, &imported.base.swc_file_path, name);

                if obj.is_none() {
                    HANDLER.with(|handler| {
                        handler.span_err(imp.range.to_span(), &format!("object not found: {name}"));
                    });
                }

                obj
            }
            ImportKind::Default => {
                let imported = self.get_or_init_module(ast_module);
                let obj =
                    imported
                        .data
                        .get_named_export(self, &imported.base.swc_file_path, "default");

                if obj.is_none() {
                    HANDLER.with(|handler| {
                        handler.span_err(imp.range.to_span(), "default export not found");
                    });
                }

                obj
            }
            ImportKind::Namespace => {
                let imported = self.get_or_init_module(ast_module);
                let obj = self.module_objects.borrow().get(&imported.base.id).cloned();

                if obj.is_none() {
                    HANDLER.with(|handler| {
                        handler.span_err(
                            imp.range.to_span(),
                            "object for namespaced import not found",
                        );
                    });
                }

                obj
            }
        }
    }
}

fn module_export_name_to_string(name: &ast::ModuleExportName) -> String {
    match name {
        ast::ModuleExportName::Ident(i) => i.sym.to_string(),
        ast::ModuleExportName::Str(str) => str.value.as_str().to_string(),
    }
}
