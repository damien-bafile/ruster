;; function
(function_item body: (_) @function.inner) @function.outer
(closure_expression body: (_) @function.inner) @function.outer

;; struct/trait/enum
(struct_item body: (_) @class.inner) @class.outer
(trait_item body: (_) @class.inner) @class.outer
(enum_item body: (_) @class.inner) @class.outer
(impl_item body: (_) @class.inner) @class.outer

;; loop
(for_expression body: (_) @loop.inner) @loop.outer
(while_expression body: (_) @loop.inner) @loop.outer
(loop_expression body: (_) @loop.inner) @loop.outer

;; parameters
(parameters) @parameter.outer
(parameters "," (_) @parameter.inner)
