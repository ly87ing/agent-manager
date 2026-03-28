#!/usr/bin/env python3
import os
import re
import shlex
import subprocess

_ENV_ASSIGNMENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=.*$")


def _starts_parameter_expansion(cmd, index):
    if cmd[index] != "$":
        return False
    if cmd.startswith("$(", index):
        return True
    if index + 1 >= len(cmd):
        return False
    next_char = cmd[index + 1]
    if next_char == "{":
        return True
    if next_char.isalnum() or next_char == "_":
        return True
    if next_char in "@*#?$!-":
        return True
    return False


def _contains_unquoted_shell_syntax(cmd):
    in_single = False
    in_double = False
    escaped = False
    index = 0

    while index < len(cmd):
        char = cmd[index]
        if escaped:
            escaped = False
            index += 1
            continue
        if char == "\\" and not in_single:
            escaped = True
            index += 1
            continue
        if char == "'" and not in_double:
            in_single = not in_single
            index += 1
            continue
        if char == '"' and not in_single:
            in_double = not in_double
            index += 1
            continue
        if not in_single and _starts_parameter_expansion(cmd, index):
            return True
        if not in_single and char == "`":
            return True
        if not in_single and not in_double:
            if cmd.startswith(("&&", "||"), index):
                return True
            if char in "|;><\n*?[":
                return True
        index += 1
    return False


def _starts_with_env_assignment(parts):
    saw_assignment = False
    for part in parts:
        if _ENV_ASSIGNMENT_RE.match(part):
            saw_assignment = True
            continue
        return saw_assignment
    return saw_assignment


def prepare_command(cmd):
    if not isinstance(cmd, str):
        return cmd, False
    if _contains_unquoted_shell_syntax(cmd):
        return cmd, True
    try:
        parts = shlex.split(cmd)
    except ValueError:
        return cmd, True
    if _starts_with_env_assignment(parts):
        return cmd, True
    return [os.path.expanduser(part) for part in parts], False


def run_command(cmd, **kwargs):
    prepared_cmd, use_shell = prepare_command(cmd)
    return subprocess.run(prepared_cmd, shell=use_shell, **kwargs)
