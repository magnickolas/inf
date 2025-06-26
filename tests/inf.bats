#!/usr/bin/env bats

INF="$BATS_TEST_DIRNAME/../inf"
ERROR_INCORRECT_ARGS=1
ERROR_NO_FILES=3
ERROR_MISSING_FILES=4

wait_for_output() {
  local log="$1" pattern="$2" count="$3" timeout="${4:-5}"
  local start
  start=$(date +%s)
  while true; do
    if [[ -f "$log" ]]; then
      local matches
      matches=$(grep -c -Fx "$pattern" "$log" 2>/dev/null || true)
      if [[ "$matches" -eq "$count" ]]; then
        return 0
      fi
    fi
    if (( $(date +%s) - start > timeout )); then
      return 1
    fi
    sleep 0.1
  done
}

quit_inf() {
  local pid="$1"
  kill -INT "$pid" 2>/dev/null || true
}

run_inf_wait() {
  local pattern="$1"
  shift
  local log="${tmpdir}/${BATS_TEST_NAME}.txt"
  "$INF" "$@" > "$log" 2>&1 &
  local pid=$!
  wait_for_output "$log" "$pattern" 1 1 || true
  quit_inf "$pid"
}

assert_log_eq() {
  local expected="$1" log="${tmpdir}/${BATS_TEST_NAME}.txt"
  diff -u <(printf '%s' "$expected") "$log" >/dev/null
}

run_inf_assert() {
  local expected="$1"
  shift
  local pattern
  pattern=$(printf '%s' "$expected" | tail -n1)
  run_inf_wait "$pattern" "$@"
  assert_log_eq "$expected"
}

run_inf_bg() {
  log="${tmpdir}/${BATS_TEST_NAME}.txt"
  "$INF" "$@" > "$log" 2>&1 &
  inf_pid=$!
}

stop_inf() {
   quit_inf "$inf_pid"
}

setup() {
  tmpdir=$(mktemp -d)
  cd "${tmpdir}" || exit 1
  touch main.c input.txt extra.txt
}

teardown() {
  rm -rf "${tmpdir}"
}

@test "no arguments prints help message" {
  run "$INF"
  [ "$status" -eq 0 ]
  [[ "$output" == Usage:* ]]
}

@test "help flag prints usage" {
  run "$INF" -h
  [ "$status" -eq 0 ]
  [[ "$output" == Usage:* ]]
}

@test "version flag prints version" {
  run "$INF" -V
  [ "$status" -eq 0 ]
  [[ "$output" == inf* ]]
}

@test "debug flag exits after printing parsed arguments" {
  run "$INF" -d -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"debug=1"* ]]
}

@test "input flag adds file to run command" {
  run "$INF" -d -r 'echo hi' -i input.txt -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"runCmd=echo hi <input.txt"* ]]
}

@test "monitor flag adds extra files" {
  run "$INF" -d -r 'echo hi' -m extra.txt -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"extra.txt"* ]]
}

@test "noparse flag disables compile file detection" {
  run "$INF" -d -n -r 'echo hi' -m extra.txt -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"monitorFiles[*]="* ]]
  mon_line=$(printf '%s\n' "$output" | grep -F 'monitorFiles[*]')
  [[ "$mon_line" != *"main.c"* ]]
}

@test "refresh flag is parsed" {
  run "$INF" -d -x -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"refresh=1"* ]]
}

@test "postpone flag is parsed" {
  run "$INF" -d -p -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"postpone=1"* ]]
}

@test "quiet flag is parsed" {
  run "$INF" -d -q -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"quiet=1"* ]]
}

@test "waitkey flag is parsed" {
  run "$INF" -d -w -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"waitkey=1"* ]]
}

@test "zen flag is parsed" {
  run "$INF" -d -z -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"zen=1"* ]]
}

@test "verbose flag is parsed" {
  run "$INF" -d -v -- gcc main.c
  [ "$status" -eq 0 ]
  [[ "$output" == *"verbose=1"* ]]
}

@test "error if incorrect arguments" {
  run "$INF" -a
  [ "$status" -eq "$ERROR_INCORRECT_ARGS" ]
  [[ "$output" == "ERROR: unknown option -a" ]]
}

@test "error if no files to monitor" {
  run "$INF" -- echo hi
  [ "$status" -eq "$ERROR_NO_FILES" ]
  [[ "$output" == *"ERROR: no files to monitor"* ]]
}

@test "error if some explicitly listed files are missing" {
  run "$INF" -m noexist.txt -- echo hi
  [ "$status" -eq "$ERROR_MISSING_FILES" ]
  [[ "$output" == *"ERROR: missing file"* ]]
}

@test "run command output is printed" {
  expected=$'[compilation: echo compile]\necho run\nrun\n[execution succeeded]\n'
  run_inf_assert "$expected" -m input.txt -r 'echo run' -- 'echo compile'
}

@test "run command output is printed in zen mode" {
  expected=$'run\n'
  run_inf_assert "$expected" -z -m input.txt -r 'echo run' -- 'echo compile'
}

@test "compile and run command output is printed in verbose mode" {
  expected=$'[compilation: echo compile]\ncompile\necho run\nrun\n[execution succeeded]\n'
  run_inf_assert "$expected" -v -m input.txt -r 'echo run' -- 'echo compile'
}

@test "compile command error output is printed" {
  expected=$'[compilation: echo compile >&2]\ncompile\necho run\nrun\n[execution succeeded]\n'
  run_inf_assert "$expected" -m input.txt -r 'echo run' -- 'echo compile >&2'
}

@test "compile command error output is not printed in quiet mode" {
  expected=$'[compilation: echo compile >&2]\necho run\nrun\n[execution succeeded]\n'
  run_inf_assert "$expected" -q -m input.txt -r 'echo run' -- 'echo compile >&2'
}

@test "compile and run commands do not run in postpone mode" {
  expected=$''
  run_inf_assert "$expected" -p -m input.txt -r 'echo run' -- 'echo compile'
}

@test "compile and run commands are run in postpone mode" {
  run_inf_bg -m input.txt -r 'echo run' -- 'echo compile'
  touch input.txt
  wait_for_output "$log" "run" 1
  stop_inf
}

@test "multiple monitor flags gather all files" {
  run_inf_bg -m input.txt -m extra.txt -r 'echo run' -- 'cat main.c >&2'
  wait_for_output "$log" "run" 1
  touch input.txt
  wait_for_output "$log" "run" 2
  touch extra.txt
  wait_for_output "$log" "run" 3
  touch dummy.txt
  wait_for_output "$log" "run" 3
  echo "maintext" > main.c
  wait_for_output "$log" "run" 4
  wait_for_output "$log" "maintext" 1
  stop_inf
}
