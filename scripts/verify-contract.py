#!/usr/bin/env python3
"""Validate package declarations using schemas from the separately pinned Core.

The SDK transport pin stays independent of newer declarative capabilities.
All schema references resolve from the checksum-verified in-memory registry;
the validator never follows a rolling main-branch reference over the network.
"""

import argparse
import hashlib
import json
from pathlib import Path
import urllib.request

from jsonschema import Draft202012Validator
from referencing import Registry, Resource


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--download", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    lock = json.loads((root / "contract.lock.json").read_text())
    revision = lock["commit"]
    assert len(revision) == 40 and all(char in "0123456789abcdef" for char in revision)
    assert lock["repository"] == "https://github.com/Lithe-IDEA/Lithe-core-rust"
    base = f"https://raw.githubusercontent.com/Lithe-IDEA/Lithe-core-rust/{revision}/shared/plugins/contract/v1/"
    cache = root / ".artifacts/contracts" / revision
    schemas = {}
    registry = Registry()
    for record in lock["schemas"]:
        name = record["path"]
        assert name in {"plugin-manifest.schema.json", "native-ui.schema.json", "plugin-ui.schema.json", "plugin-l10n.schema.json"}
        path = cache / name
        if not path.exists():
            if not args.download:
                raise RuntimeError("Missing pinned schema cache; run scripts/verify-sdk-lock.sh --download")
            with urllib.request.urlopen(base + name, timeout=30) as response:
                content = response.read(2 * 1024 * 1024 + 1)
            assert len(content) <= 2 * 1024 * 1024, "Schema exceeds the download limit"
        else:
            content = path.read_bytes()
        assert hashlib.sha256(content).hexdigest() == record["sha256"], f"Pinned schema checksum mismatch: {name}"
        schema = json.loads(content)
        Draft202012Validator.check_schema(schema)
        schemas[name] = schema
        registry = registry.with_resource(schema["$id"], Resource.from_contents(schema))
        cache.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_bytes(content)

    package = root / "io.github.lithe-idea.java-support"
    inputs = [(package / "plugin.json", "plugin-manifest.schema.json")]
    inputs.extend((path, "plugin-l10n.schema.json") for path in sorted((package / "l10n").glob("*.json")))
    for path, schema in inputs:
        validator = Draft202012Validator(schemas[schema], registry=registry)
        failures = sorted(validator.iter_errors(json.loads(path.read_text())), key=lambda error: str(error.path))
        if failures:
            details = "\n".join(f"{path.name}: {list(error.path)}: {error.message}" for error in failures)
            raise RuntimeError(details)
    print(f"Verified manifest and {len(inputs) - 1} translations against Core {revision}")


if __name__ == "__main__":
    main()
