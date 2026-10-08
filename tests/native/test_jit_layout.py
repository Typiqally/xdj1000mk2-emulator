"""Keep fitting control-flow cycles in one generated function, without losing entries."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

GENERATOR = Path(__file__).resolve().parents[2] / 'native/qemu/hw/cdj/c6x/tools/c14_jitgen.py'
spec = importlib.util.spec_from_file_location('jitgen_layout', GENERATOR)
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)


def graph(targets, size):
    decoder = SimpleNamespace(prof=[], idle_head=0, bytes32=lambda address: None)
    module = gen.ModuleGen(decoder, [], 100, 100)
    module.fn_nodes = size
    module.layout = 'loops'
    for label, target in enumerate(targets):
        state = gen.State(0x11800000 + label * 4, 0, [], [], [], 0, {})
        module.labels[state.key()] = label
        module.pos_of[label] = state
        module.entries[state.pc] = label
        module.entry_labels.add(label)
        module.code[label] = [f'L{label}: ;', module.goto(target)]
        module.cold_of[label] = []
        module.node_deps[label] = set()
    return module


class LayoutTests(unittest.TestCase):
    def test_default_preserves_discovery_order_without_a_measured_layout_gain(self):
        decoder = SimpleNamespace(prof=[], idle_head=0, bytes32=lambda address: None)
        self.assertEqual(gen.ModuleGen(decoder, [], 100, 100).layout, 'discovery')

    def test_completed_components_and_partial_bins_keep_every_state(self):
        module = graph([0, 0, 1, 4, 3], 3)
        sources, count = module.functions()
        self.assertEqual(count, 2)
        self.assertTrue(any('L3: ;' in source and 'L4: ;' in source for source in sources))
        self.assertTrue(all(sum(f'L{label}: ;' in source for label in range(5)) <= 3 for source in sources))

    def test_large_graph_does_not_depend_on_python_recursion_depth(self):
        module = graph(list(range(1, 3000)) + [0], 64)
        sources, count = module.functions()
        self.assertEqual(count, 47)
        self.assertEqual(sum(source.count(': ;') for source in sources), 3000)

    def test_discovery_option_preserves_the_baseline_partition(self):
        module = graph([2, 3, 4, 5, 0, 1], 3)
        module.layout = 'discovery'
        sources, count = module.functions()
        self.assertEqual(count, 2)
        self.assertEqual(sum(source.count('jit_hop_fn =') for source in sources), 4)

    def test_interleaved_fitting_cycles_do_not_spill_between_functions(self):
        # Discovery interleaves two independent three-state loops.
        module = graph([2, 3, 4, 5, 0, 1], 3)
        sources, count = module.functions()
        self.assertEqual(count, 2)
        self.assertEqual(sum(source.count('jit_hop_fn =') for source in sources), 0)
        for loop in ([0, 2, 4], [1, 3, 5]):
            self.assertTrue(any(all(f'L{label}: ;' in source for label in loop) for source in sources))

    def test_oversized_cycle_keeps_every_entry_and_bounds_function_size(self):
        module = graph(list(range(1, 10)) + [0], 4)
        sources, count = module.functions()
        self.assertEqual(count, 3)
        for label in range(10):
            self.assertEqual(sum(source.count(f'L{label}: ;') for source in sources), 1)
            self.assertEqual(sum(source.count(f'case {label}: goto L{label};') for source in sources), 1)
        self.assertTrue(all(sum(f'L{label}: ;' in source for label in range(10)) <= 4 for source in sources))
        self.assertEqual(sources, graph(list(range(1, 10)) + [0], 4).functions()[0])


if __name__ == '__main__':
    unittest.main()
