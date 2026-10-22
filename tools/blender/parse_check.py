"""Parse-check a python file: the proof line for a patch that only touches python.

usage: python tools/blender/parse_check.py <file> [<file> ...]
prints "parses ok <file>" for each file that parses, and exits 1 on the
first that does not.
"""
import ast
import sys


def main(argv):
    if len(argv) < 2:
        sys.stderr.write("usage: parse_check.py <file> [<file> ...]\n")
        return 2
    for path in argv[1:]:
        with open(path, "r", encoding="utf-8") as handle:
            ast.parse(handle.read(), filename=path)
        print("parses ok " + path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
