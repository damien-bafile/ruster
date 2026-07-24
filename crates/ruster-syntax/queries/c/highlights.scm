; C highlights (tree-sitter-c)

(comment) @comment
(string_literal) @string
(char_literal) @string
(system_lib_string) @string
(number_literal) @number

[
  "if" "else" "for" "while" "do"
  "switch" "case" "default"
  "break" "continue" "return" "goto"
  "typedef" "struct" "union" "enum"
  "static" "const" "extern" "volatile" "inline" "register"
  "sizeof"
] @keyword

(primitive_type) @type
(type_identifier) @type
(sized_type_specifier) @type

(call_expression function: (identifier) @function)
(function_declarator declarator: (identifier) @function)
(field_identifier) @variable
