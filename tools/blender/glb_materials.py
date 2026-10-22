"""Read a glTF/GLB JSON chunk and print the materials and the mesh primitives.

usage: python glb_materials.py <file.glb> [<file.glb> ...]
"""
import json
import struct
import sys


def read_json(path):
    with open(path, "rb") as handle:
        data = handle.read()
    magic, version, length = struct.unpack("<III", data[:12])
    if magic != 0x46546C67:
        raise SystemExit("not a glb: " + path)
    off = 12
    while off < length:
        clen, ctype = struct.unpack("<II", data[off:off + 8])
        chunk = data[off + 8:off + 8 + clen]
        if ctype == 0x4E4F534A:
            return json.loads(chunk.decode("utf-8"))
        off += 8 + clen
    raise SystemExit("no json chunk: " + path)


def main(argv):
    for path in argv[1:]:
        doc = read_json(path)
        print("== " + path)
        for i, material in enumerate(doc.get("materials", [])):
            print("  material %d %s" % (i, material.get("name")))
        for mesh in doc.get("meshes", []):
            for prim in mesh.get("primitives", []):
                print("  mesh %s -> material %s" % (mesh.get("name"), prim.get("material")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
