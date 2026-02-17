crates/core   → hawk_core  (движок: транспорт, очередь, воркер)
crates/panic  → hawk_panic (аддон: перехват паник)
crates/hawk   → hawk       (фасад: собирает всё, юзер импортит только его)

 How to create a Catcher
Introduction

If you read this page then you want to create your own catcher module for the language we do not support yet or any other custom system.
Feature requirements

This document contains the list of features that should be supported by every catcher

    Bind global error handler
    Send caught errors to the hawk using universal Event Format
    Collect and send code fragments for each line of Stacktrace
    Allow to send events to the Hawk manually 
    Allow users to specify free-format context object with any data. Context can be specified globally (on initialization) and with every manually sent event. If both present, they should be merged.
    Allow passing user object with a currently authenticated user, see Event Format. When user is not specified, the catcher should generate user with {id: 'user-unique token'} so Hawk can calculate Affected Users count.
    The catcher can pass language-specific data through the addons field
    If possible, runtime variables values should be extracted from the Stacktrace and passed to the Hawk.
    Send own version with event. Needed for source maps and suspected commits.
    If Catcher works on the backend side, it can send Suspected Commits using git.
    If possible, the Catcher should send error levels (Fatal, Warning, etc)
    If possible, the Catcher should have an interface for integration with popular loggers. For example, Monolog for PHP.