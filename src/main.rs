mod bicheck;
mod binding;
mod common;
mod error;
mod term;
use common::*;
mod codegen;
mod eval;
mod lexer;
mod query;
use rustyline as rl;

use lalrpop_util::*;
lalrpop_mod!(pub grammar);

struct Helper;

impl rl::completion::Completer for Helper {
    type Candidate = String;
}

impl rl::hint::Hinter for Helper {}

impl rl::highlight::Highlighter for Helper {}

impl rl::validate::Validator for Helper {
    fn validate(
        &self,
        ctx: &mut rl::validate::ValidationContext,
    ) -> rl::Result<rl::validate::ValidationResult> {
        if ctx.input().trim().is_empty() {
            Ok(rl::validate::ValidationResult::Incomplete)
        } else {
            let l: Vec<_> = ctx.input().lines().collect();
            if !l.first().unwrap().trim().is_empty() || ctx.input().ends_with('\n') {
                Ok(rl::validate::ValidationResult::Valid(None))
            } else {
                Ok(rl::validate::ValidationResult::Incomplete)
            }
        }
    }
}

impl rl::Helper for Helper {}

fn main() {
    // A simple REPL
    let config = rustyline::Config::builder().auto_add_history(true).build();
    let mut rl = rustyline::Editor::<Helper>::with_config(config);
    rl.set_helper(Some(Helper));

    let mut db = MainDatabase::default();
    let mut buf = String::new();
    let file = error::FILES
        .write()
        .unwrap()
        .add("<input>".to_string(), buf.clone());
    let mut seen_symbols = std::collections::HashSet::<Sym>::new();

    loop {
        let readline = rl.readline("> ");
        match readline {
            Err(_) => break,
            Ok(line) => {
                let old_buf = buf.clone();

                buf.push_str(&line);
                buf.push('\n');
                error::FILES.write().unwrap().set_source(file, buf.clone());

                db.set_source(file, std::sync::Arc::new(buf.clone()));

                for s in db.symbols(ScopeId::File(file)).iter() {
                    if !seen_symbols.contains(s) {
                        seen_symbols.insert(**s);
                        if let Some(elab) = db.elab(ScopeId::File(file), **s) {
                            let ty = elab.get_type();
                            let val = db.val(ScopeId::File(file), **s).unwrap();
                            let b = db.bindings();
                            let b = b.read().unwrap();
                            println!(
                                "{}{} : {} = {}",
                                b.resolve(**s),
                                s.num(),
                                WithContext(&b, &*ty),
                                WithContext(&b, &*val)
                            );
                        }
                    }
                }

                // Generate LLVM and print out the module, for now
                let module = db.low_mod(file);
                let context = inkwell::context::Context::create();
                let llvm = module.codegen(&mut crate::codegen::CodegenCtx::new(&context));
                llvm.print_to_stderr();
                if let Err(e) = llvm.verify() {
                    println!("Verification error: {}", e);
                }

                // If there's a `main` definition (and only one), run it and assume it returns an Int
                // This is really terrible, should definitely replace it
                let engine = llvm
                    .create_jit_execution_engine(inkwell::OptimizationLevel::None)
                    .unwrap();
                unsafe {
                    if let Ok(main) =
                        engine.get_function::<unsafe extern "C" fn() -> i32>("main$0_0")
                    {
                        println!("{}", main.call());
                    }
                }

                if db.has_errors() {
                    buf = old_buf;
                }
                db.emit_errors();
            }
        }
    }
}
