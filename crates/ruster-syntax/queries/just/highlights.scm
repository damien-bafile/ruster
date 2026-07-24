; justfile highlights (tree-sitter-just)

(comment) @comment
(string) @string
(escape_sequence) @string
(shebang) @comment

(boolean) @constant

; keywords / directives
[
  "alias"
  "export"
  "import"
  "mod"
  "set"
  "if"
  "else"
] @keyword

; recipe names and their dependencies
(recipe_header (identifier) @function)
(dependency (identifier) @function)
(alias left: (identifier) @function)

; assignments and parameters
(assignment left: (identifier) @variable)
(parameter (identifier) @variable)
(variadic_parameter (parameter (identifier) @variable))

; interpolation / function calls inside recipes
(function_call (identifier) @function)
(setting left: (identifier) @keyword)
