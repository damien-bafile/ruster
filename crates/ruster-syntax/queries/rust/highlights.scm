; Keywords
"as" @keyword
"break" @keyword
"const" @keyword
"continue" @keyword
"else" @keyword
"enum" @keyword
"extern" @keyword
"false" @keyword
"fn" @keyword
"for" @keyword
"if" @keyword
"impl" @keyword
"in" @keyword
"let" @keyword
"loop" @keyword
"match" @keyword
"mod" @keyword
"move" @keyword
"pub" @keyword
"ref" @keyword
"return" @keyword
"static" @keyword
"struct" @keyword
"trait" @keyword
"true" @keyword
"type" @keyword
"unsafe" @keyword
"use" @keyword
"where" @keyword
"while" @keyword

; Function calls
(call_expression function: (identifier) @function)
(call_expression function: (field_expression field: (field_identifier) @function.method))

; Function definitions
(function_item name: (identifier) @function)

; Type definitions
(struct_item name: (type_identifier) @type)
(enum_item name: (type_identifier) @type)
(trait_item name: (type_identifier) @type)
(type_identifier) @type

; Strings
(string_literal) @string
(char_literal) @string

; Comments
(line_comment) @comment
(block_comment) @comment

; Numbers
(integer_literal) @number
(float_literal) @number

; Operators
(assignment_expression "=" @operator)
(binary_expression ["+" "-" "*" "/" "%" "==" "!=" "<" ">" "<=" ">=" "&&" "||"] @operator)
(unary_expression ["!" "&" "*" "-"] @operator)

; Built-in types
((type_identifier) @builtin
  (#match? @builtin "^(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|char|str|String|Vec|Option|Result|Box|Rc|Arc|HashMap|HashSet)$"))
