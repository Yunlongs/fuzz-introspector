/* Copyright 2024 Fuzz Introspector Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *    http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::analyse::{CallSite, FunctionInfo};

use syn::{
    spanned::Spanned, visit::Visit, Expr, ExprCall, ExprMethodCall, ExprPath, Macro, Stmt, Path as SynPath
};

use std::collections::{HashSet, HashMap};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

pub fn generate_call_trees(
    source_dir: &str,
    functions: &[FunctionInfo],
) -> io::Result<HashMap<String, FunctionInfo>> {
    // Retrieve a list of all fuzzing harnesses
    let fuzzing_files = find_fuzzing_harnesses(source_dir)?;
    let function_map: HashMap<String, &FunctionInfo> = functions.iter().map(|f| (f.name.clone(), f)).collect();

    let mut harness_map = HashMap::new();

    // Generate call graph per harness
    for fuzz_file in &fuzzing_files {
        let harness_name = Path::new(&fuzz_file)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .replace('_', "-");

        // Prepare initials
        let output_file = format!("fuzzerLogFile-{}.data", harness_name);
        let mut output = File::create(&output_file)?;

        writeln!(output, "Call tree")?;
        writeln!(output, "fuzz_target {} linenumber=-1", fuzz_file)?;

        // Extract functions from the fuzz_target macro in the harness
        let called_functions = extract_called_functions(fuzz_file, functions)?;

        // Build the call tree
        let mut visited = HashSet::new();
        for (func_name, line_number) in &called_functions {
            if let Some(call_tree) = build_call_tree(
                &func_name,
                &function_map,
                fuzz_file,
                *line_number as i32,
                &mut visited,
                0,
            ) {
                output.write_all(call_tree.as_bytes())?;
            }
        }

        // Manually populate all fields for FunctionInfo
        let function_info = FunctionInfo {
            name: "fuzz_target".to_string(),
            file: fuzz_file.clone(),
            return_type: String::new(),
            linkage_type: String::new(),
            arg_count: 0,
            arg_names: Vec::new(),
            arg_types: Vec::new(),
            constants_touched: Vec::new(),
            called_functions: called_functions.iter().map(|(name, _)| name.clone()).collect(),
            branch_profiles: Vec::new(),
            callsites: called_functions
                .iter()
                .map(|(src, _)| CallSite {
                    src: fuzz_file.clone(),
                    dst: src.clone(),
                })
                .collect(),
            depth: 0,
            visibility: String::new(),
            icount: 0,
            bbcount: 0,
            edge_count: 0,
            complexity: 0,
            function_uses: 0,
            start_line: 0,
            end_line: 0,
        };
        harness_map.insert(fuzz_file.clone(), function_info);
    }

    Ok(harness_map)
}

// Locate all fuzzing harness files with fuzz_target macro
fn find_fuzzing_harnesses(dir: &str) -> io::Result<Vec<String>> {
    let mut harnesses = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let content = fs::read_to_string(&path)?;
            if content.contains("fuzz_target!") {
                harnesses.push(path.to_string_lossy().into_owned());
            }
        } else if path.is_dir() {
            harnesses.extend(find_fuzzing_harnesses(path.to_str().unwrap())?);
        }
    }
    Ok(harnesses)
}

// Extract all functions in the fuzz_target macro in the fuzzing harnesses
fn extract_called_functions(
    file_path: &str,
    function_info: &[FunctionInfo],
) -> io::Result<Vec<(String, usize)>> {
    let content = fs::read_to_string(file_path)?;
    let syntax = syn::parse_file(&content).expect("Failed to parse file");

    let mut visitor = FuzzTargetVisitor::new(function_info.to_vec());
    visitor.visit_file(&syntax);

    // Remove duplicate items and sort by line number
    let set: HashSet<_> = visitor.called_functions.into_iter().collect();
    let mut result: Vec<(String, usize)> = set.into_iter().collect();
    result.sort_by_key(|item| item.1);
    result = post_process_called_functions(result);

    Ok(result)
}

// Helper function to post process the called function vector
fn post_process_called_functions(items: Vec<(String, usize)>) -> Vec<(String, usize)> {
    let mut stored_value: Option<String> = None;
    let mut result = Vec::new();

    for (mut string_value, usize_value) in items {
        if let Some(pos) = string_value.rfind("::") {
            stored_value = Some(string_value[..pos].to_string());
        } else if let Some(stored) = &stored_value {
            string_value = format!("{}::{}", stored, string_value);
        }

        // Push the updated item into the result
        result.push((string_value, usize_value));
    }

    result
}

// Base struct and syn:Visit implementation for traversing the function call tree
#[derive(Default)]
struct FuzzTargetVisitor {
    called_functions: Vec<(String, usize)>,
    function_info: Vec<FunctionInfo>,
    variable_types: HashMap<String, String>,
}

impl FuzzTargetVisitor {
    pub fn new(function_info: Vec<FunctionInfo>) -> Self {
        FuzzTargetVisitor {
            called_functions: Vec::new(),
            function_info,
            variable_types: HashMap::new(),
        }
    }

    // Helper method to extract type of method call receiver
    fn extract_receiver_type(&self, receiver: &Expr) -> Option<String> {
        match receiver {
            // Variable or parameter call
            Expr::Path(path_expr) => {
                let variable_name = path_expr.path.segments.last()?.ident.to_string();
                self.variable_types.get(&variable_name).cloned()
            }

            // Chained method call
            Expr::MethodCall(method_call) => {
                let receiver_type = self.extract_receiver_type(&method_call.receiver);
                let method_name = method_call.method.to_string();
                let name = match receiver_type {
                    Some(receiver) => format!("{}::{}", receiver, method_name),
                    None => method_name.clone(),
                };
                self.lookup_function_return_type(&name)
            }

            _ => None,
        }
    }

    // Helper method to lookup function return type for reference
    fn lookup_function_return_type(&self, method_name: &str) -> Option<String> {
        let function_map: HashMap<String, &FunctionInfo> = self.function_info.iter().map(|f| (f.name.clone(), f)).collect();

        if let Some(function_info) = find_function(method_name, &function_map) {
            return Some(function_info.return_type.clone());
        }
        None
    }

    // Try extracting the local variable name creation
    fn extract_variable_name(&self, pat: &syn::Pat) -> Option<String> {
        if let syn::Pat::Ident(ident) = pat {
            Some(ident.ident.to_string())
        } else {
            None
        }
    }
}

impl<'ast> Visit<'ast> for FuzzTargetVisitor {
    // visit implementation method for locating the statement in the fuzz_target macro
    fn visit_macro(&mut self, mac: &'ast Macro) {
        if mac.path.segments.last().unwrap().ident == "fuzz_target" {
            if let Ok(body) = mac.parse_body::<Expr>() {
                self.visit_expr(&body);
            }
        }
    }

    // visit implementation method for processing each function expression
    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if let Expr::Path(ExprPath { path, .. }) = &*node.func {
            let qualified_name = path_to_string(&path);
            let line_number = node.func.span().start().line;
            self.called_functions.push((qualified_name, line_number));
        }

        for arg in &node.args {
            self.visit_expr(arg);
        }
    }

    // visit implementation method for handling echo method experssion
    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        let method_name = node.method.to_string();
        let span = node.method.span().start();
        let line_number = span.line;

        // Determine the fully qualified name
        let receiver_type = self.extract_receiver_type(&node.receiver);
        let qualified_name = match receiver_type {
            Some(receiver) => format!("{}::{}", receiver, method_name),
            None => method_name.clone(),
        };

        self.called_functions.push((qualified_name, line_number));

        self.visit_expr(&node.receiver);
        for arg in &node.args {
            self.visit_expr(arg);
        }
    }

    // visit implementation for local variables
    fn visit_local(&mut self, local: &syn::Local) {
        if let Some(init_expr) = &local.init {
            if let Some(var_name) = self.extract_variable_name(&local.pat) {
                if let Some(var_type) = self.extract_receiver_type(&init_expr.expr) {
                    self.variable_types.insert(var_name, var_type);
                }
            }
        }
    }

    // General method ensure visiting all kinds of Expr that could call functions/methods
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Call(call_expr) => {
                self.visit_expr_call(call_expr);
            }

            Expr::MethodCall(method_call_expr) => {
                self.visit_expr_method_call(method_call_expr);
            }

            Expr::Block(block_expr) => {
                for stmt in &block_expr.block.stmts {
                    match stmt {
                        Stmt::Local(local_stmt) => {
                            if let Some(init_expr) = &local_stmt.init {
                                self.visit_expr(&init_expr.expr);
                            }
                        }

                        Stmt::Expr(inner_expr, _) => {
                            self.visit_expr(inner_expr);
                        }

                        Stmt::Item(item) => {
                            syn::visit::visit_item(self, item);
                        }

                        _ => {}
                    }
                }
            }

            Expr::If(if_expr) => {
                self.visit_expr(&if_expr.cond);
                self.visit_block(&if_expr.then_branch);
                if let Some((_, else_branch)) = &if_expr.else_branch {
                    self.visit_expr(else_branch);
                }
            }

            Expr::Match(match_expr) => {
                self.visit_expr(&match_expr.expr);
                for arm in &match_expr.arms {
                    self.visit_expr(&arm.body);
                }
            }

            Expr::While(while_expr) => {
                self.visit_expr(&while_expr.cond);
                self.visit_block(&while_expr.body);
            }

            Expr::ForLoop(for_loop_expr) => {
                self.visit_expr(&for_loop_expr.expr);
                self.visit_block(&for_loop_expr.body);
            }

            Expr::Await(await_expr) => {
                self.visit_expr(&await_expr.base);
            }

            Expr::Try(try_expr) => {
                self.visit_expr(&try_expr.expr);
            }

            Expr::Closure(closure_expr) => {
                self.visit_expr(&closure_expr.body);
            }

            Expr::Return(return_expr) => {
                if let Some(inner_expr) = &return_expr.expr {
                    self.visit_expr(inner_expr);
                }
            }

            Expr::Assign(assign_expr) => {
                self.visit_expr(&assign_expr.left);
                self.visit_expr(&assign_expr.right);
            }

            Expr::Unary(unary_expr) => {
                self.visit_expr(&unary_expr.expr);
            }

            Expr::Binary(binary_expr) => {
                self.visit_expr(&binary_expr.left);
                self.visit_expr(&binary_expr.right);
            }

            Expr::Field(field_expr) => {
                self.visit_expr(&field_expr.base);
            }

            Expr::Index(index_expr) => {
                self.visit_expr(&index_expr.expr);
                self.visit_expr(&index_expr.index);
            }

            Expr::Tuple(tuple_expr) => {
                for elem in &tuple_expr.elems {
                    self.visit_expr(elem);
                }
            }

            Expr::Array(array_expr) => {
                for elem in &array_expr.elems {
                    self.visit_expr(elem);
                }
            }

            Expr::Struct(struct_expr) => {
                for field in &struct_expr.fields {
                    self.visit_expr(&field.expr);
                }
                if let Some(rest) = &struct_expr.rest {
                    self.visit_expr(rest);
                }
            }

            Expr::Paren(paren_expr) => {
                self.visit_expr(&paren_expr.expr);
            }

            Expr::Macro(macro_expr) => {
                if let Ok(parsed_body) = macro_expr.mac.parse_body::<Expr>() {
                    self.visit_expr(&parsed_body);
                }
            }

            Expr::Repeat(repeat_expr) => {
                self.visit_expr(&repeat_expr.expr);
            }

            Expr::Group(group_expr) => {
                self.visit_expr(&group_expr.expr);
            }

            _ => {
                syn::visit::visit_expr(self, expr);
            }
        }
    }
}

// Process the correct full qualified name for rust functions/methods
fn path_to_string(path: &SynPath) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

// Build and output the call tree in .data format following LLVM approach
fn build_call_tree(
    function_name: &str,
    function_map: &HashMap<String, &FunctionInfo>,
    call_path: &str,
    mut line_number: i32,
    visited: &mut HashSet<String>,
    depth: usize,
) -> Option<String> {
    let mut result = String::new();
    let indent = "  ".repeat(depth + 1);

    if line_number == 0 {
        line_number = -1;
    }

    if let Some(function_info) = find_function(function_name, function_map) {
        if visited.contains(&function_info.name) {
            return None;
        }

        visited.insert(function_info.name.clone());

        // Insert the call tree line
        result.push_str(&format!(
            "{}{} {} linenumber={}\n",
            indent, function_info.name.replace(" ", ""), call_path, line_number
        ));

        // Recursively process all function call trees
        for callsite in &function_info.callsites {
            let call_location: Vec<&str> = callsite.src.split(',').collect();
            if call_location.len() >= 2 {
                let callsite_path = call_location[0];
                let callsite_line = call_location[1].parse::<i32>().unwrap_or(-1);

                if let Some(call_tree) = build_call_tree(
                    &callsite.dst,
                    function_map,
                    callsite_path,
                    callsite_line,
                    visited,
                    depth + 1,
                ) {
                    result.push_str(&call_tree);
                }
            }
        }
    } else {
        result.push_str(&format!(
            "{}{} {} linenumber={}\n",
            indent, function_name.replace(" ", ""), call_path, line_number
        ));
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

// Search for the functions in the analysis result and exclude functions/methods not from the project
fn find_function<'a>(
    function_name: &str,
    function_map: &'a HashMap<String, &'a FunctionInfo>,
) -> Option<&'a FunctionInfo> {
    // Exact match
    if let Some(func) = function_map.get(function_name) {
        return Some(func);
    }

    // Match any key that ends with function_name
    if let Some((_, func)) = function_map.iter().find(|(key, _)| key.ends_with(function_name)) {
        return Some(func);
    }

    // Split and check segments from the right side
    let segments: Vec<&str> = function_name.split("::").collect();
    for i in 0..segments.len() {
        let partial_name = segments[i..].join("::");
        if let Some(func) = function_map.get(&partial_name) {
            return Some(func);
        }
    }

    // No match found
    None
}
