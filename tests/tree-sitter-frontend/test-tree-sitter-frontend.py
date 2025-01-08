# Copyright 2025 Fuzz Introspector Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""Unit testing script for tree-sitter-frontend."""

import os
import sys
import pytest

sys.path.append(os.path.dirname(os.path.realpath(__file__)) + "/../../")

from fuzz_introspector.frontends import oss_fuzz  # noqa: E402

def test_tree_sitter_cpp_sample1():
    callsites = oss_fuzz.analyse_folder('c++', 'cpp/test-project-1', 'LLVMFuzzerTestOneInput')

    assert len(callsites[0].split('\n')) == 6
    assert '    isPositive cpp/test-project-1/sample.cpp' in callsites[0]
