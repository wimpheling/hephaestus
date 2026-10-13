#!/usr/bin/env python3
"""Generate a static recipe from saved PublishRelease/GetRelease output.

This local generator does not authenticate publication. Installation must reload
and authorize the exact server catalog, ignoring caller-supplied publication claims.
"""
import argparse
import json
from pathlib import Path
import sys
import tomllib
import uuid

MAX_PUBLICATION_BYTES = 1048576


def identifier(value):
    if isinstance(value, dict) and set(value) == {"value"}:
        value = value["value"]
    parsed = uuid.UUID(value)
    if parsed.int == 0:
        raise ValueError("nil_release_identity")
    return str(parsed)


def input_declarations():
    config = tomllib.loads(Path(__file__).with_name("agent.toml").read_text())
    parameters = config["parameters"]
    if {parameter["name"] for parameter in parameters} != {"operation", "key", "value"}:
        raise ValueError("unsupported_package_parameters")
    declarations = []
    for parameter in parameters:
        if parameter["type"] != "string" or parameter.get("sensitive", False):
            raise ValueError("unsupported_package_parameter_type")
        declarations.append(f'''[[inputs]]
name = {json.dumps(parameter["name"])}
required = {str(parameter.get("required", False)).lower()}
default = {json.dumps(parameter["default"], ensure_ascii=False)}
value_type = {{ type = "string", minimum_length = {parameter["minimum_length"]}, maximum_length = {parameter["maximum_length"]} }}
''')
    return "\n".join(declarations)


def render(publication, *, reuse=False):
    release = publication.get("release", publication)
    if release.get("state") not in {"published", "RELEASE_STATE_PUBLISHED"}:
        raise ValueError("release_not_published")
    agents = [agent for agent in release["agents"] if agent.get("agent_key", agent.get("agentKey")) == "local-sqlite"]
    if len(agents) != 1:
        raise ValueError("local_sqlite_export_not_unique")
    release_id = identifier(release["id"])
    agent_id = identifier(agents[0]["id"])
    recipe_id = "local-sqlite-reuse" if reuse else "local-sqlite"
    source = ('{ type = "external" }' if reuse else
              '{ type = "created", capacity_bytes = { source = "literal", value = 16777216 } }')
    return f'''contract_version = 1
recipe_id = "{recipe_id}"
recipe_version = "1.0.0"

{input_declarations()}
[[resources]]
kind = "volume"
name = "data"
removal = "retain"
source = {source}

[[resources]]
kind = "instance"
name = "app"
removal = "delete"
release = {{ release_id = "{release_id}", release_agent_id = "{agent_id}" }}
parameters = {{ operation = {{ source = "input", name = "operation" }}, key = {{ source = "input", name = "key" }}, value = {{ source = "input", name = "value" }} }}

[[resources.volume_bindings]]
slot = "data"
resource = "data"
guest_path = "/data"
access_mode = "read_write"

[[outputs]]
name = "application"
value = {{ source = "resource", name = "app" }}

[[outputs]]
name = "volume"
value = {{ source = "resource", name = "data" }}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("publication_json", type=Path)
    parser.add_argument("--reuse", action="store_true", help="generate a static recipe for an externally bound retained volume")
    args = parser.parse_args()
    try:
        with args.publication_json.open("rb") as publication:
            raw = publication.read(MAX_PUBLICATION_BYTES + 1)
        if len(raw) > MAX_PUBLICATION_BYTES:
            raise ValueError("publication_too_large")
        sys.stdout.write(render(json.loads(raw), reuse=args.reuse))
        return 0
    except (ValueError, KeyError, TypeError, OSError, AttributeError):
        print("publication must contain one published local-sqlite export with real release IDs", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
