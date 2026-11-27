import unittest

from check_edges import violations


class Violations(unittest.TestCase):
    def test_clean_graph(self):
        edges = {
            "arch": {"arch-interfaces", "arch-x86_64"},
            "arch-x86_64": {"arch-interfaces", "arch-x86_64-primitives"},
            "arch-interfaces": {"boot", "kernel-types"},
            "boot": {"kernel-types"},
            "kernel-types": set(),
            "core_local": {"arch-x86_64-primitives"},
            "platform": {"platform-interfaces", "platform-pc-x86_64"},
            "kernel": {"arch", "platform", "kernel-types"},
        }
        self.assertEqual(violations(edges), [])

    def test_each_rule_fires(self):
        cases = {
            "implementation": (
                {"irq": {"arch-x86_64"}},
                ["irq depends on arch-x86_64, only arch may"],
            ),
            "interface": (
                {"arch-interfaces": {"arch"}},
                ["interface crate arch-interfaces depends on non-interface crate arch"],
            ),
            "leaf": (
                {"kernel-types": {"boot"}},
                ["leaf crate kernel-types depends on workspace crate boot"],
            ),
            "binary": (
                {"early_heap": {"kernel"}},
                ["early_heap depends on the binary crate kernel"],
            ),
            "test only": (
                {"irq": {"test-host"}},
                ["irq depends on test-host, which may only be a dev-dependency"],
            ),
            "facade": (
                {"arch": {"arch-interfaces", "arch-x86_64", "irq"}},
                ["facade arch depends on irq"],
            ),
            "interface and implementation": (
                {"platform-interfaces": {"platform-pc-x86_64"}},
                [
                    "platform-interfaces depends on platform-pc-x86_64, only platform may",
                    "interface crate platform-interfaces depends on non-interface crate "
                    "platform-pc-x86_64",
                ],
            ),
        }
        for rule, (edges, expected) in cases.items():
            with self.subTest(rule):
                self.assertEqual(violations(edges), expected)


if __name__ == "__main__":
    unittest.main()
