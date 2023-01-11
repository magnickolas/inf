# inf

A generic helper script for monitoring compilation process.

When the source files are changed, it automatically refreshes output.

<img src="https://github.com/magnickolas/inf/blob/815abc8c51ec0afb5653211c557de662dad04bb6/extra/demo.gif" width="700">

## Dependencies

- [entr](https://github.com/eradman/entr)

## Installation

Either directly download the script:
```console
wget https://raw.githubusercontent.com/magnickolas/inf/main/inf -O ~/.local/bin/inf && chmod +x ~/.local/bin/inf
```

Or install it from source: 
```console
git clone https://github.com/magnickolas/inf
cd inf
make install
inf -h # show help
```

## Usage examples

- Automatically rebuild and run a single file with provided input file
    - -v flag always prints the compiler's output (even on success); useful e.g. to see if there are warnings
```console
inf -v -i input.txt -r ./a.out -- g++ -O2 main.cpp
```

- If using `make`, we need to explicitly list the files that would trigger recompilation
    - -x flag interrupts the current running command and restart the whole process on source files change
```console
inf -x -m src/*.cpp src/*.h -r "make run" make
```

- We can use the compile phase to run some linter (in this case it's mypy for python)
    - -z flag is to print nothing but the output of the compile and run commands
```console
inf -z -r "python3 main.py" mypy main.py
```

### Notes
When `-x | --refresh` flag is passed, interactive shell is disabled due to technical reasons. If you want to enter some input, provide input file with `-i | --input`.
