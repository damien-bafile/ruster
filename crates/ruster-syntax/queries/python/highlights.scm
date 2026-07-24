; Python highlights (tree-sitter-python)

(comment) @comment
(string) @string
(integer) @number
(float) @number

[
  "def" "class" "lambda"
  "return" "yield" "pass" "break" "continue"
  "if" "elif" "else"
  "for" "while" "with" "as"
  "try" "except" "finally" "raise"
  "import" "from"
  "global" "nonlocal" "assert" "del" "await" "async"
  "and" "or" "not" "in" "is"
] @keyword

[
  "+" "-" "*" "/" "//" "%" "**"
  "=" "==" "!=" "<" ">" "<=" ">="
  "+=" "-=" "*=" "/="
  "->"
] @operator

; definitions and calls
(function_definition name: (identifier) @function)
(class_definition name: (identifier) @type)
(call function: (identifier) @function)
(call function: (attribute attribute: (identifier) @function))
(decorator (identifier) @function)
(decorator (attribute attribute: (identifier) @function))

; type annotations
(typed_parameter type: (type (identifier) @type))
(type (identifier) @type)

; keyword / default argument names
(keyword_argument name: (identifier) @variable)

[
  (true)
  (false)
  (none)
] @constant
