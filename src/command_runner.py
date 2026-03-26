#!/usr/bin/env python3
import os
import shlex
import subprocess

_SHELL_SYNTAX_MARKERS = ("&&", "||", "|", ";", "$(", "`", ">", "<", "\n")


def prepare_command(cmd):
    if not isinstance(cmd, str):
        return cmd, False
    if any(marker in cmd for marker in _SHELL_SYNTAX_MARKERS):
        return cmd, True
    return [os.path.expanduser(part) for part in shlex.split(cmd)], False


def run_command(cmd, **kwargs):
    prepared_cmd, use_shell = prepare_command(cmd)
    return subprocess.run(prepared_cmd, shell=use_shell, **kwargs)
