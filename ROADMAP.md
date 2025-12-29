# Main goal

Try to make this more useful for the general public, and not just for myself
and my colleagues.

# Issues and Ideas

[√] Garbage in output when redirecting (e.g. hitman ... | jq)
[√] Make it possible to select target without an interactive prompt (e.g. hitman -s target)
[ ] Make it possible to send requests from anywhere, without having to have a
    hitman.toml file
[ ] Make "targets" optional (default target)
[ ] Consider capturing all responses, and defining "aliases" instead of defining what to capture
[ ] Inconsistent "widgets" when using hitman (interactive) and hitman-ui. The
default interactive mode could somehow plug into hitman-ui instead of bringing
it's own widgets.

# Details: Capturing all responses

Currently, we define what to capture or extract from a response, like:

    [_extract]
    access_token = "$.result.access_token"

Instead of this, if responses are always captured, we could refer to this
directly in a different request, without having any additional configuration.
Something like this:

    Authorization: Bearer {{ @login.$.result.access_token }}

But this is quite verbose, so we could allow defining an alias as a
config variable:

    access_token = "{{ @login.$.result.access_token }}"

The advantage with this approach, is that we can work with the responses
without having to repeat the request. For instance, we could start by sending
the login request once, and then start adding other requests that refer to the
response. We could then go on to refine the structure by adding aliases and use
additional info from the response, without having to keep sending the original
login request over and over.

There are some challenges with this approach where requests "pull" data from a
specific response, rather than data being "pushed" into the state. Say we have
a "refresh_token" request where we want to overwrite the `access_token`
variable. This becomes a challenge when other requests or aliases refer
specifically to the login request.

I thought about expressing an alias chain of "OR" references, like
`token = "{{@login.token || @refresh.token}}"`, but that won't even work. Since
all responses are stored, it will keep resolving to the `@login` response even
if we get the `@refresh` response. So we would probably need to have some kind
of hybrid solution, where aliasing would work in the common case, but in some
cases where we want multiple requests to update the same state, we still need
an "extraction" system like we have now.
