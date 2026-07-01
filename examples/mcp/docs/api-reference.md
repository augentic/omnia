# Widget Service API Reference

All endpoints accept and return JSON.

## `POST /widgets`

Create a widget. The body must contain a `label` string. Returns the new
widget with its generated `id` and `state: "draft"`.

## `POST /widgets/{id}/assemble`

Assemble a `draft` widget. Fails with `409 Conflict` if the widget is not in
the `draft` state. Returns the widget with `state: "assembled"`.

## `GET /widgets/{id}/manifest`

Return the shipping manifest for an `assembled` (or `shipped`) widget. Fails
with `404 Not Found` if the widget has never been assembled.
