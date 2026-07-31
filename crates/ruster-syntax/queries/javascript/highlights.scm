; JavaScript highlights (tree-sitter-javascript)

(comment) @comment

[
  (string)
  (template_string)
] @string

(regex) @string.regex
(number) @number

[
  "function" "class" "extends" "new" "delete" "typeof" "instanceof" "void"
  "return" "yield" "throw"
  "if" "else" "switch" "case" "default"
  "for" "while" "do" "break" "continue"
  "try" "catch" "finally"
  "var" "let" "const"
  "import" "export" "from" "as"
  "async" "await"
  "static" "get" "set"
  "in" "of"
] @keyword

[
  "+" "-" "*" "/" "%" "**"
  "=" "==" "===" "!=" "!==" "<" ">" "<=" ">="
  "+=" "-=" "*=" "/=" "%=" "**="
  "&&" "||" "!" "??" "=>"
  "&" "|" "^" "~" "<<" ">>" ">>>"
  "++" "--" "..."
] @operator

; definitions
(function_declaration name: (identifier) @function)
(generator_function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function.method)
(class_declaration name: (identifier) @type)

; `const foo = () => {}` and `const foo = function () {}` read as definitions
(variable_declarator
  name: (identifier) @function
  value: [(arrow_function) (function_expression)])

; calls
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function.method))
(new_expression constructor: (identifier) @type)

; members and properties
(member_expression property: (property_identifier) @variable.member)
(pair key: (property_identifier) @variable.member)
(shorthand_property_identifier) @variable.member

; parameters
(formal_parameters (identifier) @variable.parameter)

[
  (true)
  (false)
  (null)
  (undefined)
] @constant

[
  (this)
  (super)
] @builtin
