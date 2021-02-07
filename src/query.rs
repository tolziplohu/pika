use crate::elaborate::*;
use crate::error::*;
use crate::lower::durin;
use crate::term::*;
use std::sync::{Arc, Mutex};

macro_rules! intern_id {
    ($name:ident, $doc:expr) => {
        #[doc=$doc]
        #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(salsa::InternId);
        impl $name {
            /// Returns the underlying number identifying the object, for debugging purposes
            pub fn num(self) -> u32 {
                self.0.as_u32()
            }
        }
        impl salsa::InternKey for $name {
            fn from_intern_id(id: salsa::InternId) -> Self {
                Self(id)
            }

            fn as_intern_id(&self) -> salsa::InternId {
                self.0
            }
        }
    };
    ($name:ident) => {
        intern_id!($name, "This is a handle to the actual object stored in the Salsa database.")
    }
}

intern_id!(Name, "An identifier, represented as an interned string.");
impl Name {
    pub fn get<T: Interner + ?Sized>(self, db: &T) -> String {
        db.lookup_intern_name(self)
    }
}
intern_id!(
    DefId,
    "A reference to a definition and the context, which are interned in the Salsa database."
);
intern_id!(
    PreDefId,
    r#"A reference to a definition, but without context.
This is needed for (mutually) recursive definitions, where context for one definition requires the others."#
);
intern_id!(
    ScopeId,
    "A reference to a scope with members, for a datatype's associated module or a structure."
);
intern_id!(
    TypeId,
    "A reference to a datatype and its constructors, but not its associated module."
);
intern_id!(
    Cxt,
    r#"The context for resolving names, represented as a linked list of definitions, with the links stored in Salsa.
This is slower than a hashmap or flat array, but it has better incrementality."#
);
impl Cxt {
    pub fn size<T: ?Sized + Interner>(self, db: &T) -> Lvl {
        match db.lookup_cxt_entry(self) {
            MaybeEntry::Yes(CxtEntry { size, .. }) => size,
            MaybeEntry::No(_) => Lvl::zero(),
        }
    }

    pub fn env<T: ?Sized + Interner>(self, db: &T) -> Env {
        Env::new(self.size(db))
    }

    pub fn file<T: ?Sized + Interner>(self, db: &T) -> FileId {
        match db.lookup_cxt_entry(self) {
            MaybeEntry::Yes(CxtEntry { file, .. }) => file,
            MaybeEntry::No(file) => file,
        }
    }

    pub fn lookup(self, sym: Name, db: &dyn Compiler) -> Result<(Var<Ix>, VTy), DefError> {
        let mut cxt = self;
        let mut ix = Ix::zero();
        while let MaybeEntry::Yes(CxtEntry {
            name, info, rest, ..
        }) = db.lookup_cxt_entry(cxt)
        {
            match info {
                NameInfo::Def(id) => {
                    if name == sym {
                        return Ok((Var::Top(id), (*db.def_type(id)?).clone()));
                    }
                }
                NameInfo::Local(ty) => {
                    if name == sym {
                        return Ok((Var::Local(ix), ty));
                    } else {
                        ix = ix.inc()
                    }
                }
                NameInfo::Rec(id, ty) => {
                    if name == sym {
                        return Ok((Var::Rec(id), ty));
                    }
                }
                NameInfo::Other(v, ty) => {
                    if name == sym {
                        return Ok((v, ty));
                    }
                }
            }
            cxt = rest;
        }
        Err(DefError::NoValue)
    }

    pub fn lookup_rec(self, rec: PreDefId, db: &dyn Compiler) -> Option<DefId> {
        let mut cxt = self;
        while let MaybeEntry::Yes(CxtEntry { info, rest, .. }) = db.lookup_cxt_entry(cxt) {
            match info {
                NameInfo::Def(id) => {
                    if db.lookup_intern_def(id).0 == rec {
                        return Some(id);
                    }
                }
                NameInfo::Rec(id, _) => {
                    if id == rec {
                        return None;
                    }
                }
                NameInfo::Local(_) | NameInfo::Other(_, _) => (),
            }
            cxt = rest;
        }
        None
    }

    #[must_use]
    pub fn define<T: ?Sized + Interner>(self, name: Name, info: NameInfo, db: &T) -> Cxt {
        let (file, size) = match db.lookup_cxt_entry(self) {
            MaybeEntry::Yes(CxtEntry { file, size, .. }) => (file, size),
            MaybeEntry::No(file) => (file, Lvl::zero()),
        };
        let next_size = match &info {
            NameInfo::Local(_) => size.inc(),
            _ => size,
        };
        db.cxt_entry(MaybeEntry::Yes(CxtEntry {
            name,
            info,
            file,
            size: next_size,
            rest: self,
        }))
    }

    pub fn new<T: ?Sized + Interner>(file: FileId, db: &T) -> Cxt {
        let cxt = db.cxt_entry(MaybeEntry::No(file));
        crate::builtins::define_builtins(cxt, db)
    }
}

#[derive(Clone, Eq, PartialEq, Debug, Hash)]
pub enum RecSolution {
    Defined(PreDefId, u16, Span, Val),
    Inferred(PreDefId, u16, Span, Val),
}
impl RecSolution {
    pub fn id(&self) -> PreDefId {
        match self {
            RecSolution::Defined(id, _, _, _) | RecSolution::Inferred(id, _, _, _) => *id,
        }
    }

    pub fn num(&self) -> u16 {
        match self {
            RecSolution::Defined(_, n, _, _) | RecSolution::Inferred(_, n, _, _) => *n,
        }
    }

    pub fn val(&self) -> &Val {
        match self {
            RecSolution::Defined(_, _, _, v) | RecSolution::Inferred(_, _, _, v) => v,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum NameInfo {
    Def(DefId),
    Local(VTy),
    Rec(PreDefId, VTy),
    Other(Var<Ix>, VTy),
}

/// One cell of the context linked list.
/// See `Interner::cxt_entry`.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct CxtEntry {
    pub name: Name,
    pub info: NameInfo,
    pub file: FileId,
    pub size: Lvl,
    pub rest: Cxt,
}
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum MaybeEntry {
    Yes(CxtEntry),
    No(FileId),
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ElabInfo {
    pub term: Arc<Term>,
    pub typ: Arc<VTy>,
    pub solved_globals: Arc<Vec<RecSolution>>,
    /// Only used for definitions in the associated namespace of a datatype
    pub children: Arc<Vec<DefId>>,
}

#[salsa::query_group(InternerDatabase)]
pub trait Interner: salsa::Database {
    #[salsa::interned]
    fn intern_name(&self, s: String) -> Name;

    #[salsa::interned]
    fn intern_predef(&self, def: Arc<PreDefAn>) -> PreDefId;

    #[salsa::interned]
    fn intern_def(&self, def: PreDefId, cxt: Cxt, eff_stack: Option<EffStack>) -> DefId;

    #[salsa::interned]
    fn intern_scope(&self, scope: Arc<Vec<(Name, DefId)>>) -> ScopeId;

    #[salsa::interned]
    fn intern_type(&self, constructors: Arc<Vec<(Name, DefId)>>) -> TypeId;

    /// The context is stored as a linked list, but the links are hashmap keys!
    /// This is... pretty slow, but has really good incrementality.
    #[salsa::interned]
    fn cxt_entry(&self, key: MaybeEntry) -> Cxt;
}

pub trait CompilerExt: Interner {
    fn report_error(&self, error: Error);

    fn num_errors(&self) -> usize;

    fn write_errors(&self);
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DefError {
    /// This happens when we can't find a variable, or we try to get the value of a definition that doesn't have one, like a declaration.
    NoValue,
    ElabError(DefId),
}

#[salsa::query_group(CompilerDatabase)]
pub trait Compiler: CompilerExt + Interner {
    #[salsa::input]
    fn file_source(&self, file: FileId) -> String;

    /// Returns a list of the interned defs
    fn top_level(&self, file: FileId) -> Arc<Vec<DefId>>;

    fn elaborate_def(&self, def: DefId) -> Result<ElabInfo, DefError>;

    fn def_type(&self, def: DefId) -> Result<Arc<VTy>, DefError>;

    fn durin(&self, file: FileId) -> durin::ir::Module;
}

fn def_type(db: &dyn Compiler, def: DefId) -> Result<Arc<VTy>, DefError> {
    db.elaborate_def(def).map(|ElabInfo { typ, .. }| typ)
}

fn top_level(db: &dyn Compiler, file: FileId) -> Arc<Vec<DefId>> {
    use crate::grammar::DefsParser;

    let source = db.file_source(file);

    let parser = DefsParser::new();
    let cxt = Cxt::new(file, db);
    match parser.parse(db, crate::lexer::Lexer::new(&source)) {
        Ok(v) => Arc::new(intern_block(v, db, cxt, None)),
        Err(e) => {
            db.report_error(e.to_error(file));
            Arc::new(Vec::new())
        }
    }
}

pub fn intern_block(v: Vec<PreDefAn>, db: &dyn Compiler, mut cxt: Cxt, eff_stack: Option<EffStack>) -> Vec<DefId> {
    let mut rv = Vec::new();
    // This stores unordered definitions (types and functions) between local variables
    let mut temp = Vec::new();
    for def in v {
        match &*def {
            // Unordered
            PreDef::Fun(_, _, _, _)
            | PreDef::Type(_, _, _, _, _)
            | PreDef::FunDec(_, _, _)
            | PreDef::ValDec(_, _) => {
                let name = def.name();
                let def = Arc::new(def);
                let id = db.intern_predef(def.clone());
                if let Some(name) = name {
                    if let Some(ty) = def.given_type(id, cxt, db) {
                        cxt = cxt.define(name, NameInfo::Rec(id, ty), db);
                    } else {
                        cxt = cxt.define(name, NameInfo::Rec(id, Val::Error), db);
                    }
                }
                temp.push((name, id));
            }
            // Ordered
            PreDef::Val(_, _, _) | PreDef::Impl(_, _, _) | PreDef::Expr(_) | PreDef::Cons(_, _) => {
                // Process `temp` first
                for (name, pre) in temp.drain(0..) {
                    let id = db.intern_def(pre, cxt, eff_stack.clone());
                    if let Some(name) = name {
                        // Define it for real now
                        cxt = cxt.define(name, NameInfo::Def(id), db);
                    }
                    rv.push(id);
                }

                // Then add this one
                let name = def.name();
                let pre = db.intern_predef(Arc::new(def));
                let id = db.intern_def(pre, cxt, eff_stack.clone());
                if let Some(name) = name {
                    cxt = cxt.define(name, NameInfo::Def(id), db);
                }
                rv.push(id);
            }
        }
    }
    // If anything is left in `temp`, clear it out
    for (name, pre) in temp {
        let id = db.intern_def(pre, cxt, eff_stack.clone());
        if let Some(name) = name {
            // Define it for real now
            cxt = cxt.define(name, NameInfo::Def(id), db);
        }
        rv.push(id);
    }
    rv
}

#[salsa::database(InternerDatabase, CompilerDatabase)]
#[derive(Default)]
pub struct Database {
    storage: salsa::Storage<Self>,
    errors: Mutex<Vec<Error>>,
}

impl salsa::Database for Database {}

impl CompilerExt for Database {
    fn report_error(&self, error: Error) {
        self.errors.lock().unwrap().push(error);

        use salsa::Database;
        // Make sure whatever query reported an error runs again next time
        // We need it to report the error again even if nothing changed
        self.salsa_runtime().report_untracked_read();
    }

    fn num_errors(&self) -> usize {
        self.errors.lock().unwrap().len()
    }

    fn write_errors(&self) {
        for err in self.errors.lock().unwrap().drain(0..) {
            err.write().unwrap();
        }
    }
}

impl Database {
    pub fn check_all(&self, file: FileId) {
        // TODO: get meta solutions from each elaborate_def() and make sure they match
        for def in &*self.top_level(file) {
            // They already reported any errors to the database, so we ignore them here
            let _ = self.elaborate_def(*def);
        }
    }
}
