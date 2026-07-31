; TypeScript / TSX highlights (tree-sitter-typescript, TSX dialect)
;
; The parser is registered as LANGUAGE_TSX (see lib.rs), so this must stay valid
; against the tsx grammar — it is a superset of the ts one.

(comment) @comment

[
  (string)
  (template_string)
] @string

(regex) @string.regex
(number) @number

[
  "function" "class" "extends" "implements" "new" "delete" "typeof"
  "instanceof" "void" "keyof" "infer" "is" "asserts" "satisfies"
  "return" "yield" "throw"
  "if" "else" "switch" "case" "default"
  "for" "while" "do" "break" "continue"
  "try" "catch" "finally"
  "var" "let" "const"
  "import" "export" "from" "as"
  "async" "await"
  "static" "get" "set"
  "interface" "type" "enum" "namespace" "declare" "abstract"
  "public" "private" "protected" "readonly"
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

; types
(type_identifier) @type
(predefined_type) @type.builtin
(interface_declaration name: (type_identifier) @type)
(type_alias_declaration name: (type_identifier) @type)
(enum_declaration name: (identifier) @type)

; definitions
(function_declaration name: (identifier) @function)
(generator_function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function.method)
(class_declaration name: (type_identifier) @type)

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
(property_signature name: (property_identifier) @variable.member)

; parameters
(required_parameter pattern: (identifier) @variable.parameter)
(optional_parameter pattern: (identifier) @variable.parameter)

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
