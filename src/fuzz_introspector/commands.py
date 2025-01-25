# Copyright 2022 Fuzz Introspector Authors
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
"""High-level routines and CLI entrypoints"""

import logging
import os
import json
import yaml
import shutil
from typing import Optional

from fuzz_introspector import analysis
from fuzz_introspector import constants
from fuzz_introspector import diff_report
from fuzz_introspector import html_helpers
from fuzz_introspector import html_report
from fuzz_introspector import utils

from fuzz_introspector.frontends import oss_fuzz

logger = logging.getLogger(name=__name__)


def diff_two_reports(report1: str, report2: str) -> int:
    diff_report.diff_two_reports(report1, report2)
    return constants.APP_EXIT_SUCCESS


def correlate_binaries_to_logs(binaries_dir: str) -> int:
    pairings = utils.scan_executables_for_fuzz_introspector_logs(binaries_dir)
    logger.info("Pairings: %s", str(pairings))
    with open("exe_to_fuzz_introspector_logs.yaml", "w+") as etf:
        etf.write(yaml.dump({'pairings': pairings}))
    return constants.APP_EXIT_SUCCESS


def end_to_end(args) -> int:
    """Runs both frontend and backend."""
    if not args.language:
        args.language = utils.detect_language(args.target_dir)

    if args.out_dir:
        out_dir = args.out_dir
    else:
        out_dir = os.getcwd()

    if not os.path.exists(out_dir):
        os.mkdir(out_dir)

    if args.language == constants.LANGUAGES.JAVA:
        entrypoint = 'fuzzerTestOneInput'
    else:
        entrypoint = 'LLVMFuzzerTestOneInput'

    oss_fuzz.analyse_folder(language=args.language,
                            directory=args.target_dir,
                            entrypoint=entrypoint,
                            out=out_dir)

    if 'c' in args.language:
        language = 'c-cpp'
    else:
        language = args.language

    correlation_file = os.path.join(out_dir,
                                    'exe_to_fuzz_introspector_logs.yaml')
    if not os.path.isfile(correlation_file):
        correlation_file = ''

    return run_analysis_on_dir(target_folder=out_dir,
                               coverage_url=args.coverage_url,
                               analyses_to_run=[],
                               correlation_file=correlation_file,
                               enable_all_analyses=True,
                               report_name=args.name,
                               language=language,
                               out_dir=out_dir)


def run_analysis_on_dir(target_folder: str,
                        coverage_url: str,
                        analyses_to_run: list[str],
                        correlation_file: str,
                        enable_all_analyses: bool,
                        report_name: str,
                        language: str,
                        output_json: Optional[list[str]] = None,
                        parallelise: bool = True,
                        dump_files: bool = True,
                        out_dir: str = '') -> int:
    """Runs Fuzz Introspector analysis from based on the results
    from a frontend run. The primary task is to aggregate the data
    and generate a HTML report."""

    constants.should_dump_files = dump_files

    if enable_all_analyses:
        for analysis_interface in analysis.get_all_analyses():
            if analysis_interface.get_name() not in analyses_to_run:
                analyses_to_run.append(analysis_interface.get_name())

    introspection_proj = analysis.IntrospectionProject(language, target_folder,
                                                       coverage_url)
    introspection_proj.load_data_files(parallelise, correlation_file, out_dir)

    logger.info("Analyses to run: %s", str(analyses_to_run))
    logger.info("[+] Creating HTML report")
    if output_json is None:
        output_json = []
    html_report.create_html_report(introspection_proj,
                                   analyses_to_run,
                                   output_json,
                                   report_name,
                                   dump_files,
                                   out_dir=out_dir)

    return constants.APP_EXIT_SUCCESS


def light_analysis(args) -> int:
    """Performs a light analysis, without any data from the frontends, so
    no compilation is needed for this analysis."""
    src_dir = os.getenv('SRC', '/src/')
    inspector_dir = os.path.join(src_dir, 'inspector')
    light_dir = os.path.join(inspector_dir, 'light')

    if not os.path.isdir(light_dir):
        os.makedirs(light_dir, exist_ok=True)

    all_tests = analysis.extract_tests_from_directories({src_dir},
                                                        args.language,
                                                        inspector_dir)

    with open(os.path.join(light_dir, 'all_tests.json'), 'w') as f:
        f.write(json.dumps(list(all_tests)))

    pairs = analysis.light_correlate_source_to_executable(args.language)
    with open(os.path.join(light_dir, 'all_pairs.json'), 'w') as f:
        f.write(json.dumps(list(pairs)))

    all_source_files = analysis.extract_all_sources(args.language)
    light_out_src = os.path.join(light_dir, 'source_files')

    for source_file in all_source_files:
        dst = light_out_src + '/' + source_file
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copy(source_file, dst)
    with open(os.path.join(light_dir, 'all_files.json'), 'w') as f:
        f.write(json.dumps(list(all_source_files)))

    return 0


def analyse(args) -> int:
    """Perform a light analysis using the chosen Analyser and return
    json results."""
    # Retrieve the correct analyser
    target_analyser = None
    for analyser in analysis.get_all_standalone_analyses():
        if analyser.get_name() == args.analyser:
            target_analyser = analysis.instantiate_analysis_interface(analyser)
            break

    # Return error if analyser not found
    if not target_analyser:
        logger.error('Analyser %s not found.', args.analyser)
        return constants.APP_EXIT_ERROR

    # Auto detect project language is not provided
    if not args.language:
        args.language = utils.detect_language(args.target_dir)

    # Prepare out directory
    if args.out_dir:
        out_dir = args.out_dir
    else:
        out_dir = os.getcwd()

    if not os.path.exists(out_dir):
        os.mkdir(out_dir)

    # Fix entrypoint default for languages
    if args.language == constants.LANGUAGES.JAVA:
        entrypoint = 'fuzzerTestOneInput'
    else:
        entrypoint = 'LLVMFuzzerTestOneInput'

    # Run the frontend
    oss_fuzz.analyse_folder(language=args.language,
                            directory=args.target_dir,
                            entrypoint=entrypoint,
                            out=out_dir)

    if 'c' in args.language:
        language = 'c-cpp'
    else:
        language = args.language

    # Perform the FI backend project analysis from the frontend
    introspection_proj = analysis.IntrospectionProject(language, out_dir, '')
    introspection_proj.load_data_files(True, '', out_dir)

    # Perform specific actions for certain standalone analyser
    if target_analyser.get_name() == 'SourceCodeLineAnalyser':
        source_file = args.source_file
        source_line = args.source_line

        target_analyser.set_source_file_line(source_file, source_line)
    elif target_analyser.get_name() == 'FarReachLowCoverageAnalyser':
        exclude_static_functions = args.exclude_static_functions
        only_referenced_functions = args.only_referenced_functions
        only_header_functions = args.only_header_functions
        only_interesting_functions = args.only_interesting_functions
        max_functions = args.max_functions

        introspection_proj.load_debug_report(out_dir)

        target_analyser.set_flags(exclude_static_functions,
                                  only_referenced_functions,
                                  only_header_functions,
                                  only_interesting_functions)
        target_analyser.set_max_functions(max_functions)
        target_analyser.set_introspection_project(introspection_proj)

    # Run the analyser
    target_analyser.analysis_func(html_helpers.HtmlTableOfContents(), [],
                                  introspection_proj.proj_profile,
                                  introspection_proj.profiles, '', '', [],
                                  out_dir)

    return constants.APP_EXIT_SUCCESS
