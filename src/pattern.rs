use crate::common::*;
use crate::elaborate::*;
use crate::term::*;
use bit_set::BitSet;
use durin::ir::Width;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Pat {
    Any,
    Var(Name, Box<VTy>),
    Cons(DefId, Box<VTy>, Vec<Pat>),
    Or(Box<Pat>, Box<Pat>),
    Lit(Literal, Width),
    Bool(bool),
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum Cov {
    /// We covered everything, there's nothing left
    All,
    /// We didn't cover anything yet
    None,
    /// We *did* cover these constructors
    Cons(Vec<(DefId, Vec<Cov>)>),
    /// We *did* cover these literals
    Lit(BitSet),
    /// we *did* cover this Bool
    Bool(bool),
}
impl Cov {
    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Cov::All, _) | (_, Cov::All) => Cov::All,
            (Cov::None, x) | (x, Cov::None) => x,
            (Cov::Bool(x), Cov::Bool(y)) if x != y => Cov::All,
            (Cov::Bool(x), Cov::Bool(_)) => Cov::Bool(x),
            (Cov::Lit(mut a), Cov::Lit(b)) => {
                a.union_with(&b);
                Cov::Lit(a)
            }
            (Cov::Cons(mut v), Cov::Cons(v2)) => {
                for (cons, cov) in v2 {
                    if let Some((_, cov2)) = v.iter_mut().find(|(c, _)| *c == cons) {
                        *cov2 = std::mem::take(cov2)
                            .into_iter()
                            .zip(cov)
                            .map(|(x, y)| x.or(y))
                            .collect();
                    } else {
                        v.push((cons, cov));
                    }
                }
                Cov::Cons(v)
            }
            _ => unreachable!(),
        }
    }

    pub fn pretty_rest(&self, ty: &VTy, db: &dyn Compiler, mcxt: &MCxt) -> Doc {
        match self {
            Cov::All => Doc::start("<nothing>"),
            Cov::None => Doc::start("_"),
            // We don't show what literals we've covered.
            // In the future, we may switch to ranges like Rust uses.
            Cov::Lit(_) => Doc::start("_"),
            Cov::Bool(b) => Doc::start(match b {
                // What's *uncovered*?
                false => "True",
                true => "False",
            }),
            Cov::Cons(covs) => match ty {
                Val::App(Var::Type(_, sid), _, _, _) => {
                    let mut v = Vec::new();
                    let mut unmatched: Vec<DefId> = db
                        .lookup_intern_scope(*sid)
                        .iter()
                        .filter_map(|&(_name, id)| {
                            let info = db.elaborate_def(id).ok()?;
                            match &*info.term {
                                Term::Var(Var::Cons(cid), _) if id == *cid => {
                                    let cty = IntoOwned::<Val>::into_owned(info.typ)
                                        .ret_type(mcxt.size, mcxt, db);
                                    if crate::elaborate::local_unify(
                                        cty,
                                        ty.clone(),
                                        mcxt.size,
                                        Span::empty(),
                                        db,
                                        &mut mcxt.clone(),
                                    )
                                    .ok()?
                                    {
                                        Some(id)
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            }
                        })
                        .collect();

                    for (cons, args) in covs {
                        unmatched.retain(|id| id != cons);
                        if args.iter().any(|x| *x != Cov::All) {
                            let (pre, _) = db.lookup_intern_def(*cons);
                            let pre = db.lookup_intern_predef(pre);
                            let name = pre.name().unwrap();

                            let mut cons_ty = (*db
                                .elaborate_def(*cons)
                                .expect("probably an invalid constructor?")
                                .typ)
                                .clone();
                            let mut l = mcxt.size;

                            let mut v2 = vec![Doc::start(name.get(db))];
                            for x in args {
                                let ty = match cons_ty {
                                    Val::Fun(from, to) => {
                                        cons_ty = *to;
                                        *from
                                    }
                                    Val::Pi(_, cl) => {
                                        let from = cl.ty.clone();
                                        cons_ty = cl.vquote(l.inc(), mcxt, db);
                                        l = l.inc();
                                        from
                                    }
                                    _ => unreachable!(),
                                };
                                v2.push(x.pretty_rest(&ty, db, mcxt));
                            }

                            v.push(Doc::intersperse(v2, Doc::none().space()));
                        }
                    }

                    for cons in unmatched {
                        let (pre, _) = db.lookup_intern_def(cons);
                        let pre = db.lookup_intern_predef(pre);
                        let name = pre.name().unwrap();

                        let mut cons_ty = (*db
                            .elaborate_def(cons)
                            .expect("probably an invalid constructor?")
                            .typ)
                            .clone();
                        let mut l = mcxt.size;

                        let mut v2 = vec![Doc::start(name.get(db))];
                        loop {
                            match cons_ty {
                                Val::Fun(_, to) => {
                                    cons_ty = *to;
                                }
                                Val::Pi(_, to) => {
                                    cons_ty = to.vquote(l.inc(), mcxt, db);
                                    l = l.inc();
                                }
                                _ => break,
                            };
                            v2.push(Doc::start("_"));
                        }

                        v.push(Doc::intersperse(v2, Doc::none().space()));
                    }

                    Doc::intersperse(v, Doc::start("; "))
                }
                _ => unreachable!(),
            },
        }
    }

    pub fn simplify(self, ty: &VTy, db: &dyn Compiler, mcxt: &MCxt) -> Self {
        match self {
            Cov::All => Cov::All,
            Cov::None => Cov::None,
            Cov::Lit(s) => Cov::Lit(s),
            Cov::Bool(x) => Cov::Bool(x),
            Cov::Cons(mut covs) => match ty {
                Val::App(Var::Type(_, sid), _, _, _) => {
                    let mut unmatched: Vec<DefId> = db
                        .lookup_intern_scope(*sid)
                        .iter()
                        .filter_map(|&(_name, id)| {
                            let info = db.elaborate_def(id).ok()?;
                            match &*info.term {
                                Term::Var(Var::Cons(cid), _) if id == *cid => {
                                    let cty = IntoOwned::<Val>::into_owned(info.typ)
                                        .ret_type(mcxt.size, mcxt, db);
                                    if crate::elaborate::local_unify(
                                        cty,
                                        ty.clone(),
                                        mcxt.size,
                                        Span::empty(),
                                        db,
                                        &mut mcxt.clone(),
                                    )
                                    .ok()?
                                    {
                                        Some(id)
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            }
                        })
                        .collect();
                    for (cons, args) in &mut covs {
                        let mut cons_ty = (*db
                            .elaborate_def(*cons)
                            .expect("probably an invalid constructor?")
                            .typ)
                            .clone();
                        let mut l = mcxt.size;

                        for x in std::mem::take(args) {
                            let ty = match cons_ty {
                                Val::Fun(from, to) => {
                                    cons_ty = *to;
                                    *from
                                }
                                Val::Pi(_, cl) => {
                                    let from = cl.ty.clone();
                                    cons_ty = cl.vquote(l.inc(), mcxt, db);
                                    l = l.inc();
                                    from
                                }
                                _ => unreachable!(),
                            };
                            args.push(x.simplify(&ty, db, mcxt));
                        }

                        if args.iter().all(|x| *x == Cov::All) {
                            // This pattern is guaranteed to cover this constructor completely
                            unmatched.retain(|id| id != cons);
                        }
                    }

                    if unmatched.is_empty() {
                        Cov::All
                    } else {
                        Cov::Cons(covs)
                    }
                }
                _ => panic!(
                    "Called Cov::simplify() on a Cov::Cons but passed non-datatype {:?}",
                    ty
                ),
            },
        }
    }
}

impl Pat {
    pub fn cov(&self) -> Cov {
        match self {
            Pat::Any => Cov::All,
            Pat::Var(_, _) => Cov::All,
            Pat::Cons(id, _, v) => Cov::Cons(vec![(*id, v.into_iter().map(Pat::cov).collect())]),
            Pat::Or(x, y) => x.cov().or(y.cov()),
            Pat::Lit(l, _) => Cov::Lit(std::iter::once(l.to_usize()).collect()),
            Pat::Bool(b) => Cov::Bool(*b),
        }
    }

    pub fn pretty(&self, db: &dyn Compiler, names: &mut Names) -> Doc {
        match self {
            Pat::Any => Doc::start("_"),
            Pat::Var(n, _ty) => {
                let n = names.disamb(*n, db);
                names.add(n);
                Doc::start(n.get(db))
            }
            Pat::Cons(id, _, p) => Doc::start(
                db.lookup_intern_predef(db.lookup_intern_def(*id).0)
                    .name()
                    .unwrap()
                    .get(db),
            )
            .chain(Doc::intersperse(
                p.iter()
                    .map(|x| Doc::none().space().chain(x.pretty(db, names))),
                Doc::none(),
            )),
            Pat::Or(x, y) => x
                .pretty(db, names)
                .space()
                .add('|')
                .space()
                .chain(y.pretty(db, names)),
            Pat::Lit(x, _) => x.pretty(),
            Pat::Bool(b) => Doc::start(match b {
                true => "True",
                false => "False",
            }),
        }
    }

    pub fn add_locals(&self, mcxt: &mut MCxt, db: &dyn Compiler) {
        match self {
            Pat::Any | Pat::Lit(_, _) | Pat::Bool(_) => {}
            Pat::Var(n, ty) => mcxt.define(*n, NameInfo::Local((**ty).clone()), db),
            Pat::Cons(_, _, v) => {
                for p in v {
                    p.add_locals(mcxt, db);
                }
            }
            Pat::Or(_, _) => {
                // Right now we don't allow bindings in or-patterns
                ()
            }
        }
    }

    pub fn add_names(&self, l: Lvl, names: &mut Names) -> Lvl {
        match self {
            Pat::Any | Pat::Lit(_, _) | Pat::Bool(_) => l,
            Pat::Var(n, _) => {
                names.add(*n);
                l.inc()
            }
            Pat::Cons(_, _, v) => v.iter().fold(l, |l, p| p.add_names(l, names)),
            Pat::Or(_, _) => l,
        }
    }

    pub fn inline_metas(self, mcxt: &MCxt, db: &dyn Compiler) -> Self {
        match self {
            Pat::Any => Pat::Any,
            Pat::Var(n, mut t) => {
                *t = t.inline_metas(mcxt, db);
                Pat::Var(n, t)
            }
            Pat::Cons(x, mut ty, y) => {
                *ty = ty.inline_metas(mcxt, db);
                Pat::Cons(
                    x,
                    ty,
                    y.into_iter().map(|x| x.inline_metas(mcxt, db)).collect(),
                )
            }
            Pat::Or(mut x, mut y) => {
                *x = x.inline_metas(mcxt, db);
                *y = y.inline_metas(mcxt, db);
                Pat::Or(x, y)
            }
            Pat::Lit(x, w) => Pat::Lit(x, w),
            Pat::Bool(x) => Pat::Bool(x),
        }
    }

    pub fn match_with(
        &self,
        val: &Val,
        mut env: Env,
        mcxt: &MCxt,
        db: &dyn Compiler,
    ) -> Option<Env> {
        match self {
            Pat::Any => Some(env),
            Pat::Var(_, _) => {
                env.push(Some(val.clone()));
                Some(env)
            }
            Pat::Cons(id, _, v) => match val.clone().inline(env.size, db, mcxt) {
                Val::App(Var::Cons(id2), _, sp, _) => {
                    if *id == id2 {
                        for (i, (_, val)) in v.iter().zip(&sp) {
                            env = i.match_with(val, env, mcxt, db)?;
                        }
                        Some(env)
                    } else {
                        None
                    }
                }
                _ => unreachable!(),
            },
            Pat::Lit(x, _) => match val.unarc() {
                Val::Lit(l, _) => {
                    if l == x {
                        Some(env)
                    } else {
                        None
                    }
                }
                _ => unreachable!(),
            },
            Pat::Bool(x) => match val.unarc() {
                Val::App(Var::Builtin(Builtin::True), _, _, _) => {
                    if *x {
                        Some(env)
                    } else {
                        None
                    }
                }
                Val::App(Var::Builtin(Builtin::False), _, _, _) => {
                    if !x {
                        Some(env)
                    } else {
                        None
                    }
                }
                _ => unreachable!(),
            },
            Pat::Or(x, y) => x
                .match_with(val, env.clone(), mcxt, db)
                .or_else(|| y.match_with(val, env, mcxt, db)),
        }
    }
}

pub fn elab_case(
    value: Term,
    vspan: Span,
    val_ty: VTy,
    reason: ReasonExpected,
    cases: &[(Pre, Pre)],
    mut ret_ty: Option<(VTy, ReasonExpected)>,
    mcxt: &mut MCxt,
    db: &dyn Compiler,
) -> Result<(Term, VTy), TypeError> {
    let state = mcxt.state();
    let mut rcases = Vec::new();
    let mut last_cov = Cov::None;

    let mut first = true;
    for (pat, body) in cases {
        let pat = elab_pat(pat, &val_ty, reason.clone(), mcxt, db)?;
        let body = match &mut ret_ty {
            Some((ty, reason)) => {
                let term = check(body, &ty, reason.clone(), db, mcxt)?;
                // If the type we were given is a meta, the actual reason for future type errors is the type of the first branch
                if first {
                    if let Val::App(Var::Meta(_), _, _, _) = &ty {
                        *reason = ReasonExpected::MustMatch(body.span());
                    }
                }
                term
            }
            None => {
                let (term, ty) = infer(true, body, db, mcxt)?;
                ret_ty = Some((ty, ReasonExpected::MustMatch(body.span())));
                term
            }
        };
        first = false;

        mcxt.set_state(state.clone());
        let cov = last_cov.clone().or(pat.cov()).simplify(&val_ty, db, mcxt);
        if cov == last_cov {
            // TODO real warnings
            eprintln!("warning: redundant pattern");
        }
        last_cov = cov;

        rcases.push((pat, body));
    }

    if last_cov == Cov::All {
        let vty = val_ty.quote(mcxt.size, mcxt, db);
        Ok((
            Term::Case(Box::new(value), Box::new(vty), rcases),
            ret_ty.unwrap().0,
        ))
    } else {
        Err(TypeError::Inexhaustive(vspan, last_cov, val_ty))
    }
}

pub fn elab_pat(
    pre: &Pre,
    ty: &VTy,
    reason: ReasonExpected,
    mcxt: &mut MCxt,
    db: &dyn Compiler,
) -> Result<Pat, TypeError> {
    match &**pre {
        Pre_::Lit(l) => match ty {
            Val::App(Var::Builtin(b), _, _, _) => match b {
                Builtin::I32 => Ok(Pat::Lit(*l, Width::W32)),
                Builtin::I64 => Ok(Pat::Lit(*l, Width::W64)),
                _ => Err(TypeError::InvalidPatternBecause(Box::new(
                    TypeError::NotIntType(pre.span(), ty.clone().inline_metas(mcxt, db), reason),
                ))),
            },
            _ => Err(TypeError::InvalidPatternBecause(Box::new(
                TypeError::NotIntType(pre.span(), ty.clone().inline_metas(mcxt, db), reason),
            ))),
        },
        Pre_::Var(n) => {
            if let Ok((Var::Top(id), _)) = mcxt.lookup(*n, db) {
                if let Ok(info) = db.elaborate_def(id) {
                    if let Term::Var(Var::Cons(id2), _) = &*info.term {
                        if id == *id2 {
                            // This is a constructor
                            return elab_pat_app(pre, VecDeque::new(), ty, reason, mcxt, db);
                        }
                    }
                }
            }
            if let Ok((Var::Builtin(b), _)) = mcxt.lookup(*n, db) {
                match b {
                    Builtin::True => {
                        if !unify(
                            ty.clone(),
                            Val::builtin(Builtin::Bool, Val::Type),
                            mcxt.size,
                            pre.span(),
                            db,
                            mcxt,
                        )
                        .map_err(|x| TypeError::InvalidPatternBecause(Box::new(x)))?
                        {
                            return Err(TypeError::InvalidPatternBecause(Box::new(
                                TypeError::Unify(
                                    mcxt.clone(),
                                    pre.copy_span(Val::builtin(Builtin::Bool, Val::Type)),
                                    ty.clone().inline_metas(mcxt, db),
                                    reason,
                                ),
                            )));
                        }
                        return Ok(Pat::Bool(true));
                    }
                    Builtin::False => {
                        if !unify(
                            ty.clone(),
                            Val::builtin(Builtin::Bool, Val::Type),
                            mcxt.size,
                            pre.span(),
                            db,
                            mcxt,
                        )
                        .map_err(|x| TypeError::InvalidPatternBecause(Box::new(x)))?
                        {
                            return Err(TypeError::InvalidPatternBecause(Box::new(
                                TypeError::Unify(
                                    mcxt.clone(),
                                    pre.copy_span(Val::builtin(Builtin::Bool, Val::Type)),
                                    ty.clone().inline_metas(mcxt, db),
                                    reason,
                                ),
                            )));
                        }
                        return Ok(Pat::Bool(false));
                    }
                    _ => (),
                }
            }
            mcxt.define(*n, NameInfo::Local(ty.clone()), db);
            Ok(Pat::Var(*n, Box::new(ty.clone())))
        }
        Pre_::Hole(MetaSource::Hole) => Ok(Pat::Any),
        Pre_::App(_, _, _) => {
            /// Find the head and spine of an application
            fn sep(pre: &Pre) -> (&Pre, VecDeque<(Icit, &Pre)>) {
                match &**pre {
                    Pre_::App(i, f, x) => {
                        let (head, mut spine) = sep(f);
                        spine.push_back((*i, x));
                        (head, spine)
                    }
                    _ => (pre, VecDeque::new()),
                }
            }
            let (head, spine) = sep(pre);

            elab_pat_app(head, spine, ty, reason, mcxt, db)
        }
        Pre_::Dot(head, member, spine) => elab_pat_app(
            &pre.copy_span(Pre_::Dot(head.clone(), member.clone(), Vec::new())),
            spine.iter().map(|(i, x)| (*i, x)).collect(),
            ty,
            reason,
            mcxt,
            db,
        ),
        Pre_::OrPat(x, y) => {
            let size_before = mcxt.size;
            let x = elab_pat(x, ty, reason.clone(), mcxt, db)?;
            if mcxt.size != size_before {
                todo!("error: for now we don't support capturing inside or-patterns")
            }
            let y = elab_pat(y, ty, reason, mcxt, db)?;
            if mcxt.size != size_before {
                todo!("error: for now we don't support capturing inside or-patterns")
            }

            Ok(Pat::Or(Box::new(x), Box::new(y)))
        }
        _ => Err(TypeError::InvalidPattern(pre.span())),
    }
}

fn elab_pat_app(
    head: &Pre,
    mut spine: VecDeque<(Icit, &Pre)>,
    expected_ty: &VTy,
    reason: ReasonExpected,
    mcxt: &mut MCxt,
    db: &dyn Compiler,
) -> Result<Pat, TypeError> {
    let span = Span(
        head.span().0,
        spine.back().map(|(_, x)| x.span()).unwrap_or(head.span()).1,
    );

    let (term, mut ty) = infer(false, head, db, mcxt)?;
    let mut l = mcxt.size;
    match term.inline_top(db) {
        Term::Var(Var::Cons(id), _) => {
            let mut pspine = Vec::new();

            let arity = ty.arity(true);
            let f_arity = spine.iter().filter(|(i, _)| *i == Icit::Expl).count();

            while let Some((i, pat)) = spine.pop_front() {
                match ty {
                    Val::Pi(Icit::Impl, cl) if i == Icit::Expl => {
                        // Add an implicit argument to the pattern, and keep the explicit one on the stack
                        spine.push_front((i, pat));
                        mcxt.define(
                            db.intern_name("_".into()),
                            NameInfo::Local(cl.ty.clone()),
                            db,
                        );
                        pspine.push(Pat::Var(cl.name, Box::new(cl.ty.clone())));
                        ty = cl.vquote(l.inc(), mcxt, db);
                        l = l.inc();
                    }
                    Val::Pi(i2, cl) if i == i2 => {
                        let pat = elab_pat(pat, &cl.ty, reason.clone(), mcxt, db)?;
                        pspine.push(pat);
                        ty = cl.vquote(l.inc(), mcxt, db);
                        l = l.inc();
                    }
                    Val::Fun(from, to) if i == Icit::Expl => {
                        let pat = elab_pat(pat, &from, reason.clone(), mcxt, db)?;
                        pspine.push(pat);
                        ty = *to;
                    }
                    _ => return Err(TypeError::WrongNumConsArgs(span, arity, f_arity)),
                }
            }

            // Apply any remaining implicits
            loop {
                match ty {
                    Val::Pi(Icit::Impl, cl) => {
                        mcxt.define(
                            db.intern_name("_".into()),
                            NameInfo::Local(cl.ty.clone()),
                            db,
                        );
                        pspine.push(Pat::Var(cl.name, Box::new(cl.ty.clone())));
                        ty = cl.vquote(l.inc(), mcxt, db);
                        l = l.inc();
                    }
                    ty2 => {
                        // Put it back
                        ty = ty2;
                        break;
                    }
                }
            }

            match &ty {
                Val::Fun(_, _) | Val::Pi(_, _) => {
                    return Err(TypeError::WrongNumConsArgs(span, arity, f_arity))
                }
                ty => {
                    // Unify with the expected type, for GADTs and constructors of different datatypes
                    match crate::elaborate::local_unify(
                        ty.clone(),
                        expected_ty.clone(),
                        l,
                        span,
                        db,
                        mcxt,
                    ) {
                        Ok(true) => (),
                        Ok(false) => {
                            return Err(TypeError::InvalidPatternBecause(Box::new(
                                TypeError::Unify(
                                    mcxt.clone(),
                                    Spanned::new(ty.clone().inline_metas(mcxt, db), span),
                                    expected_ty.clone().inline_metas(mcxt, db),
                                    reason,
                                ),
                            )))
                        }
                        Err(e) => return Err(TypeError::InvalidPatternBecause(Box::new(e))),
                    }
                }
            }

            Ok(Pat::Cons(id, Box::new(expected_ty.clone()), pspine))
        }
        _ => Err(TypeError::InvalidPattern(span)),
    }
}