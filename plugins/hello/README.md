# Hello plugin

The smallest Patinae reference plugin. It registers the `hello` command and an
integer setting, illustrating the plugin declaration and command interfaces.

## Build and install

From the repository root, using the same revision as the Patinae application:

```sh
cargo build --locked --release -p hello-plugin
mkdir -p ~/.patinae/plugins
cp target/release/libhello_plugin.dylib ~/.patinae/plugins/
```

On Linux copy `libhello_plugin.so`; on Windows copy `hello_plugin.dll`.
Restart Patinae. See the [plugin overview](../README.md) for custom directories
and building all plugins together.

## Usage

Enter these commands in the Patinae REPL:

```pml
hello
hello Alice
```

Output:

```text
Greetings from plug-in system, World!
Greetings from plug-in system, Alice!
```

`hello [name]` uses `World` when no name is supplied. `help hello` displays the
command help; `capabilities plugins` confirms that the plugin loaded.

## Setting example

```pml
get hello_style
set hello_style, 1
get hello_style
```

`hello_style` is an integer with default `0`. It demonstrates registration and
the shared `set`/`get` interface. The greeting does not currently read this
setting, so changing it does not change the output.

## Source

[src/lib.rs](src/lib.rs) contains the complete plugin: the settings declaration,
registration macro, command implementation, and help text. To build your own
plugin from this example, follow the [authoring guide](../../docs/make-your-own-plugin.md).
