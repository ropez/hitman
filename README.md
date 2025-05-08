# HITMAN

A command line tool for hitting API endpoints.

## Basic usage

Create a file like `request.http` containing a literal HTTP request, with
headers and optional body. Then send the request by invoking hitman:

```
$ hitman request.http
```

## Project setup

To unleash the full power of hitman, a project setup is required. A hitman "project" is a
directory that contains a single hitman configuration file, `hitman.toml`. Inside the
project directory, there can be many HTTP or GraphQL request files, or subdirectories that contain
request files.

Create your project folder, containing a TOML config file, and HTTP request files
for each API you want to hit.

Example layout:

```
project
├── hitman.toml
├── login.http
├── apple/get_apples.http
├── apple/post_new_apple.http
├── apple/delete_apple.http
```

The HTTP files are templates for literal HTTP requests. Variables in double
curly braces will be substituted with values from the config file.

```
POST {{base_url}}/login HTTP/1.1
Content-Type: application/json

{
    "username": "{{api_username}}",
    "password": "{{api_password}}"
}
```

The configuration file can contain global default variables, and target
specific variables. It must contain at least one target, as a TOML table:

```toml
api_username = "admin"

[default]
base_url = "http://example.com"

[development]
base_url = "http://localhost:8080"
```

In addition to the main configuration file `hitman.toml`, there can be another
called `hitman.local.toml`. The recommended setup, is to have a `hitman.toml`
in a shared repository, and have a git ignored `hitman.local.toml` where each
team member can have their personal credentials and such.

Substitutions can be nested, so that variables can contain references to other
variables. For example:

```
authorization_header: "Authorization: Bearer {{auth_token}}"
```

Be careful, since there is currently no protection against cyclic references,
something like `foo: "{{foo}}"` will likely overflow and crash.

## Running

First, select which target to use:

```
$ hitman --select

? Select target ›
  default
  development
```

Then run requests directly by passing a request file:

```
$ hitman login.http
```

Or, use the interactive mode:

```
$ hitman

? Make request ›
  login.http
  apple/get_apples.http
  apple/post_new_apple.http
  apple/delete_apple.http
```

## Capturing responses

The core concept of HITMAN is to extract values from responses, so that they
can be referred to in templates, and substituted in subsequent requests.

A typical use-case, is to capture a token from a login response, and use it
in the authorization header in subsequent requests:

We can define how values are to be extracted in the main config file, but more
typically, we can define them specific to one request. We add request-specific
configuration by creating a file with the same name as the request file, with
`.http.toml` extension.

The `_extract` section defines which values are to be extracted from the
response, as JSON-path expressions.

```toml
# login.http.toml

[_extract]
access_token = "$.result.access_token"
refresh_token = "$.result.refresh_token"
```

When receiving a successful login response, these values are extracted, and
saved as configuration variables, which can be used in other requests:

```
GET {{base_url}}/apple HTTP/1.1
Authorization: Bearer {{access_token}}
```

## Fallback values

A variable expression can have a default value, denoted by a pipe character:
`{{user_id | 1000}}`.

When executing this request, if there is no value for `user_id` in scope,
hitman will use the fallback value, dependig on how it was invoked.

By default, hitman will ask the user to input a value. The prompt will show to
the user that there is a default value, which will be used if the user simple
hits Enter, instead of the empty string.

When run in a non-interactive mode (`-n`, `--flurry` etc), the fallback value
will be used without prompting the user, unless a value is specified in the
config, or given on the command line.

## List value selection

It's possible to specify multiple values for a variable in the config file, as
a TOML array. By default, hitman will prompt the user to select a value from a
menu, using arrow keys or fuzzy search.

Each value in the array can be specified as a simple scalar value (string,
number etc), or each can be given as a table containing a `name` and `value`.
The `name` will be shown to the user for selection, and `value` will be
inserted into the request. A typical use-case for this, is when an API uses
numerical IDs, which are inconvenient for selection.

Lists of substitution values can also be extracted from responses. Consider an
API that has an end-point like `GET /apples` and `GET /apple/{{apple_id}}`. To
be able to call the second end-point, we can set up the first to extract a list
of all available ID's. The syntax for this extraction is as follows:

```toml
[_extract]
apple_id = { _ = "$", name = "$.name", value = "$.id" }
```

There are three JSON-path expressions. The key `_` specifies how to extract the
list from the response. It's assumed to point to a JSON array. In this case,
the symbol `$` simply means that the whole response should contain and array.
In other cases, we might need something like `$.items`, `$.data` etc.

The other JSON-paths, `name` and `value` refer to data within each object of
the array.

## Flurry rush attack

It's possible to use hitman for simple performance/stress testing an API. This
is done by giving `--flurry N` on the command line, where `N` is the number of
requests to send. In this mode, interactive prompts are currently not
supported, so it can only be used when all substitutions are available in
scope, have fallback values, or are given on the command line.

By default, it uses 10 parallel connections to execute `N` requests as quickly
as possible. It's possible to specify the number of connections with the
`--connections` option. For instance, `--flurry 100 --connections 100` will try
to send all 100 requests in parallel.

## Watch mode

There is a `--watch` option that will keep hitman watching for file changes,
and re-execute the same request every time a file is changed. The `http`
request file itself, and the different config files are all watched for
changes.

Currently, the 'data' file that is updated when hitman extracts variables from
requests, is not watched, because it might create infinite loops.
