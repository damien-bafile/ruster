; YAML highlights (tree-sitter-yaml)

(comment) @comment

(block_mapping_pair key: (flow_node) @keyword)
(flow_mapping (flow_pair key: (flow_node) @keyword))

(double_quote_scalar) @string
(single_quote_scalar) @string
(block_scalar) @string

(boolean_scalar) @constant
(null_scalar) @constant
(integer_scalar) @number
(float_scalar) @number

(anchor_name) @variable
(alias_name) @variable
(tag) @type
