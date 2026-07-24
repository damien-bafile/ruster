; Scheme highlights (tree-sitter-scheme)

(comment) @comment
(block_comment) @comment
(string) @string
(escape_sequence) @string
(number) @number
(keyword) @keyword
(directive) @keyword

[
  (boolean)
  (character)
] @constant

; the first symbol of a list is in call/operator position
(list . (symbol) @function)
