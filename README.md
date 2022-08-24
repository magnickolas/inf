# inf

A helper script for monitoring compilation process.
When the source files are changed, it automatically refreshes output.

## Requirements

- Bash
- [entr](https://github.com/eradman/entr)

## Quickstart

- Automatically rebuild and run a single file with provided input file
```shell
inf -r ./a.out -i a.in -- g++ -O2 a.cpp
 ```

- In case of using *make* one has to provide the list of files that will trigger recompilation
```shell
echo src/*.cpp src/*.h | inf -r "make run" make
```

- For interpretable languages one can use either compilation or run commands (in the second case the name of the file for monitoring should be listed explicitly)
```shell
inf python3 main.py
inf -m main.py -r "python3 main.py"
```

## Flags

| Flag            |     Value format      | Description                                                                                 |
| --------------- |:---------------------:| ------------------------------------------------------------------------------------------- |
| -r \| --run     |       <command>       | Target execution command                                                                    |
| -i \| --input   |        <name>         | Input file                                                                                  |
| -m \| --monitor | <name 1>,<name 2>,... | Extra comma-separated files to trigger recompilation, the flag may be passed multiple times |
| -n \| --noparse |           —           | Don't look for \*.\* names patterns in compile command                                      |
| -v \| --verbose |           —           | Always output STDOUT from compilation command                                               |
| -h \| --help    |           —           | Help message                                                                                |
| -d \| --debug   |           —           | Print and validate parsed arguments                                                         |
