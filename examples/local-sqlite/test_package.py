import copy
from pathlib import Path
import tomllib
import unittest
import uuid

import render_recipe


class PackageTests(unittest.TestCase):
    def publication(self):
        # Test-only generated IDs; these are not a published distribution recipe.
        return {"release": {"id": {"value": str(uuid.uuid4())}, "state": "RELEASE_STATE_PUBLISHED", "agents": [{"id": {"value": str(uuid.uuid4())}, "agent_key": "local-sqlite"}]}}

    def test_generated_recipe_uses_exact_publication_ids_and_safe_retention(self):
        publication = self.publication()
        recipe = tomllib.loads(render_recipe.render(publication))
        volume, instance = recipe["resources"]
        self.assertEqual(instance["release"]["release_id"], publication["release"]["id"]["value"])
        self.assertEqual(instance["release"]["release_agent_id"], publication["release"]["agents"][0]["id"]["value"])
        self.assertEqual(volume["removal"], "retain")
        self.assertEqual(instance["volume_bindings"], [{"slot": "data", "resource": "data", "guest_path": "/data", "access_mode": "read_write"}])

    def test_inputs_are_bounded_package_defaults_and_parameter_references(self):
        config = tomllib.loads(Path(__file__).with_name("agent.toml").read_text())
        recipe = tomllib.loads(render_recipe.render(self.publication()))
        self.assertEqual(recipe["inputs"], [{
            "name": parameter["name"], "required": parameter.get("required", False),
            "default": parameter["default"], "value_type": {
                "type": parameter["type"], "minimum_length": parameter["minimum_length"],
                "maximum_length": parameter["maximum_length"],
            },
        } for parameter in config["parameters"]])
        self.assertEqual(recipe["resources"][1]["parameters"], {
            name: {"source": "input", "name": name} for name in ("operation", "key", "value")
        })

    def test_reuse_profile_is_distinct_static_external_and_retained(self):
        publication = self.publication()
        created = tomllib.loads(render_recipe.render(publication))
        reused = tomllib.loads(render_recipe.render(publication, reuse=True))
        self.assertEqual(created["recipe_id"], "local-sqlite")
        self.assertEqual(reused["recipe_id"], "local-sqlite-reuse")
        self.assertEqual(reused["resources"][0]["source"], {"type": "external"})
        self.assertEqual(reused["resources"][0]["removal"], "retain")
        self.assertEqual(created["inputs"], reused["inputs"])
        self.assertEqual(created["resources"][1], reused["resources"][1])

    def test_unpublished_nil_or_ambiguous_export_cannot_generate_pins(self):
        publication = self.publication()
        invalid = [dict(publication["release"], state="draft")]
        nil = copy.deepcopy(publication)
        nil["release"]["id"]["value"] = str(uuid.UUID(int=0))
        invalid.append(nil)
        duplicate = copy.deepcopy(publication)
        duplicate["release"]["agents"] *= 2
        invalid.append(duplicate)
        for value in invalid:
            with self.assertRaises(ValueError):
                render_recipe.render(value)

    def test_release_contract_is_named_mount_without_legacy_state_or_network(self):
        config = tomllib.loads(Path(__file__).with_name("agent.toml").read_text())
        self.assertFalse(config["state_volume"]["enabled"])
        self.assertFalse(config["workspace"]["mount"])
        self.assertEqual(config["network"]["profile"], "disabled")
        self.assertEqual(config["build"]["network"]["profile"], "disabled")
        self.assertEqual(config["volume_slots"], [{"slot": "data", "guest_path": "/data", "access_mode": "read_write", "required": True, "minimum_capacity_bytes": 16777216}])


if __name__ == "__main__":
    unittest.main()
