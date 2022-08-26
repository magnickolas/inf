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
inf -x -r ./a.out -i a.in -- g++ -O2 a.cpp
 ```

- In case of using `make` one has to provide the list of files that will trigger recompilation
```console
echo src/*.cpp src/*.h | inf -x -r "make run" make
```

- For interpretable languages one can execute some linter as a compilation command
```console
inf -r "python3 main.py" mypy main.py
```

## Flags

| Flag              |     Value format      | Description                                                                                 |
| ----------------- |:---------------------:| ------------------------------------------------------------------------------------------- |
| `-r \| --run`     |       `command`       | Target execution command                                                                    |
| `-i \| --input`   |        `name`         | Input file                                                                                  |
| `-m \| --monitor` | `name 1,<name 2>,...` | Extra files to trigger recompilation                                                        |
| `-n \| --noparse` |           —           | Don't look for \*.\* names patterns in compile command                                      |
| `-x \| --refresh` |           —           | Restart compilation immediately on files change                                             |
| `-v \| --verbose` |           —           | Always output STDOUT from compilation command                                               |
| `-h \| --help`    |           —           | Help message                                                                                |
| `-d \| --debug`   |           —           | Print and validate parsed arguments                                                         |

### Notes
When `-x | --refresh` option is enabled, interactive shell is disabled. One has to provide input file name with `-i | --input` if the run command expects some input data.
