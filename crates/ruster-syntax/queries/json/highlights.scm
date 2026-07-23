; JSON highlights (tree-sitter-json)

(string) @string
(number) @number
(escape_sequence) @constant

; object keys highlighted as keywords, values as strings
(pair key: (string) @keyword)

[
  (true)
  (false)
  (null)
] @constant
