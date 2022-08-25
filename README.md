# inf

A helper script for monitoring compilation process.
When the source files are changed, it automatically refreshes output.

<img src="https://github.com/magnickolas/inf/blob/815abc8c51ec0afb5653211c557de662dad04bb6/extra/demo.gif" width="700">

## Dependencies

- [entr](https://github.com/eradman/entr)
- Bash

## Installation

Clone repository
```console
git clone --recursive https://github.com/magnickolas/inf
```

Then either install dependencies manually and only install inf
```console
make install
```
or build and install them together
```console
make install_deps install 
```

## Quickstart

- Automatically rebuild and run a single file with provided input file
```console
inf -r ./a.out -i a.in -- g++ -O2 a.cpp
 ```

- In case of using *make* one has to provide the list of files that will trigger recompilation
```console
echo src/*.cpp src/*.h | inf -r "make run" make
```

- For interpretable languages one can use either compilation or run commands (in the second case the name of the file for monitoring should be listed explicitly)
```console
inf python3 main.py
inf -m main.py -r "python3 main.py"
```

## Flags

| Flag              |     Value format      | Description                                                                                 |
| ----------------- |:---------------------:| ------------------------------------------------------------------------------------------- |
| `-r \| --run`     |       <command>       | Target execution command                                                                    |
| `-i \| --input`   |        <name>         | Input file                                                                                  |
| `-m \| --monitor` | <name 1>,<name 2>,... | Extra files to trigger recompilation                                                        |
| `-n \| --noparse` |           —           | Don't look for \*.\* names patterns in compile command                                      |
| `-v \| --verbose` |           —           | Always output STDOUT from compilation command                                               |
| `-h \| --help`    |           —           | Help message                                                                                |
| `-d \| --debug`   |           —           | Print and validate parsed arguments                                                         |
