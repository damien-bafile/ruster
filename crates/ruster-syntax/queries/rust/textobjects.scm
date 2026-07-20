; function.inner
(function_item body: (block) @function.inner)

; function.outer
(function_item) @function.outer

; class.inner / struct body
(struct_item body: (field_declaration_list) @class.inner)

; class.outer / struct item
(struct_item) @class.outer

; loop.inner
(for_expression body: (block) @loop.inner)
(loop_expression body: (block) @loop.inner)
(while_expression body: (block) @loop.inner)

; loop.outer
(for_expression) @loop.outer
(loop_expression) @loop.outer
(while_expression) @loop.outer

; parameter.inner
(parameters (parameter) @parameter.inner)

; parameter.outer
(parameters) @parameter.outer
