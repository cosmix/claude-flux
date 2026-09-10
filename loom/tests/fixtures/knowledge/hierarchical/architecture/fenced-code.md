## Before

This real section comes before two fenced examples.

```text
## Hidden backtick heading
This line is code, not a markdown section.
```

<!-- Tilde fences intentionally test heading detection with both fence styles. -->
<!-- markdownlint-disable MD048 -->

~~~text
## Hidden tilde heading
This line is also code, not a markdown section.
~~~

<!-- markdownlint-enable MD048 -->

## After

This real section comes after both fenced examples.
