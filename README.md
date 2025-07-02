# inf

<img height="128" alt="inf logo" src="https://raw.githubusercontent.com/magnickolas/inf/refs/heads/main/extra/logo.svg" align="left"> *Instant feedback for your development loop.*

inf monitors source files and executes the given compile and run commands as soon as those files change. It is essentially a wrapper around [entr][entr] that provides convenience flags for common development workflows.

<img src="https://raw.githubusercontent.com/magnickolas/inf/refs/heads/main/extra/demo.gif" width="700">

## Dependencies

- [entr][entr]

## Installation

Either simply download the latest release:
```console
wget https://github.com/magnickolas/inf/releases/latest/download/inf -O ~/.local/bin/inf && chmod +x ~/.local/bin/inf
```

Or download the latest version from master branch:
```console
wget https://raw.githubusercontent.com/magnickolas/inf/main/inf -O ~/.local/bin/inf && chmod +x ~/.local/bin/inf
```

Or install it from source: 
```console
git clone https://github.com/magnickolas/inf
cd inf
make install prefix=~/.local exec=inf
```

## Usage examples

Rebuild and run when `main.c` changes:
```console
inf --run ./main gcc -o main main.c
```

Pipe input into the binary when either `main.c` or `input.txt` changes:
```console
inf --input input.txt --run ./main gcc -o main main.c
```

For build systems, list every source file that should trigger a rebuild. Here the shell expands the globs and pipes them into inf (inf monitors all `*.c` and `*.h` files in `src/`):
```console
echo src/*.c src/*.h | inf --run "make test" make -j4
```

- Run a static type checker in _zen_ mode (no meta-headers), whenever any Python file in `src/` or its subdirectories changes:

```console
inf -z mypy src/**/*.py
```

### Notes
When `-x | --refresh` is used, interactive shell is disabled due to technical reasons. If you want to pass some input, provide an input file with `-i | --input`.

[entr]: https://github.com/eradman/entr
