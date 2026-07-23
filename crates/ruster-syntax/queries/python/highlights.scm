; Minimal Python highlights (tree-sitter-python)

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
  "global" "nonlocal" "assert" "del"
  "and" "or" "not" "in" "is"
] @keyword

[
  "+" "-" "*" "/" "//" "%" "**"
  "=" "==" "!=" "<" ">" "<=" ">="
  "+=" "-=" "*=" "/="
] @operator

(function_definition name: (identifier) @function)
(call function: (identifier) @function)
(call function: (attribute attribute: (identifier) @function))

(class_definition name: (identifier) @type)

(decorator) @function

[
  (true)
  (false)
  (none)
] @constant
