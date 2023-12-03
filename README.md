# HITMAN

A command line tool for hitting API endpoints, without bullshit.

## Basic setup

Create your project folder, containing a TOML config file, and HTTP request files
for each API request you want to hit.

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
POST {{BaseUrl}}/login HTTP/1.1
Content-Type: application/json

{
    "username": "{{Username}}",
    "password": "{{Password}}"
}
```

The configuration file can contain global default variables, and target
specific variables. It must contain at least one target, as a TOML table:

```toml
Username = "admin"

[default]
BaseUrl = "http://example.com"

[development]
BaseUrl = "http://localhost:8080"
```

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
response, as JSONPATH expressions.

```toml
# login.http.toml

[_extract]
AccessToken = "$.result.access_token"
RefreshToken = "$.result.refresh_token"
```

When receiving a successful login response, these values are extracted, and
saved as configuration variables, which be used in other requests:

```
GET {{BaseUrl}}/apple HTTP/1.1
Authorization: Bearer {{AccessToken}}
```

## Fallback values

TODO

## List substitution

TODO

