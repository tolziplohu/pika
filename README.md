# Pika
Pika is a small, dependently typed ML with algebraic effects and unboxed types.
This is the rewritten version of the compiler, and the new typechecker is heavily inspired by [smalltt](https://github.com/AndrasKovacs/smalltt).

Currently, Pika can compile dependently-typed lambda calculus to LLVM (through [Durin](https://github.com/tolziplohu/durin), a dependently typed optimizing intermediate language) and thus native code, but it doesn't have all its planned features implemented yet.

### Example
Pika doesn't have all its planned features implemented yet, but here are some that currently work.
Look in the `tests` folder for more examples of Pika code that works today.
For a demonstration of planned features, see `demo.pk`.
```
# Syntax is similar to Standard ML, but comments use #
# Pika doesn't have universes, so Type has type Type
val U : Type = Type

# Functions can have implicit parameters with []
fun id [T] (x : T) : T = x

# And types can be inferred
fun one (x : Type) = x
fun two y = one y

# You can pass implicit parameters implicitly or explicitly
val test = id Type
val test2 = id [Type] Type

# And you can use explicit lambdas instead of `fun`
# Also, `_` introduces a hole filled by unification
val id2 : [T] T -> _ = \x. x

# Pika has datatypes and pattern matching as well
type Option T of
  Some T
  None
where
  # This is in Option's "associated namespace", as are the constructors, like `impl` in Rust.
  # Code outside of the associated namespace needs to qualify the constructors when matching, like `Option.None`
  fun unwrap_or [T] (self : Option T) (default : T) = case self of
    Some x => x
    None => default
  end
end
val _ = Option.unwrap_or (Option.Some Type) (Type -> Type)

# And algebraic effects
# Of course, this example would be better if we had strings
eff Console of
  Print I32 : ()
  Read () : I32
end

fun greet () : () with Console = do
  Console.Print 1
  val name : I32 = Console.Read ()
  Console.Print name
end

fun main () : () = do
  fun handle (x : () with Console) : () = case x of
    () => ()
    eff (Console.Print _) k => handle (k ()?)
    eff (Console.Read ()) k => handle (k 12?)
  end

  # ? stops effect propagation so it can be match on
  # It's the opposite of ? in Rust
  handle (greet ()?)
end
```

#### Why "Pika"?
Lots of languages have animal mascots, so Pika's is the pika.
Pikas are little mammals that live on mountains, close relatives of rabbits.
Pika the language is also small, but it isn't a close relative of any rabbits.
Since it has dependent types, it has pi types, and "Pika" has "Pi" in it, so that's something else.
