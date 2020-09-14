const prod = process.env.NODE_ENV === "production"
const path = require("path");
const HtmlWebpackPlugin = require("html-webpack-plugin");
const webpack = require("webpack");
const WasmPackPlugin = require("@wasm-tool/wasm-pack-plugin");
const CopyWebpackPlugin = require("copy-webpack-plugin");

module.exports = {
    context: path.join(__dirname, "."),
    entry: "./index.js",
    output: {
        path: path.resolve(__dirname, "dist"),
        filename: "index.js",
    },
    plugins: [
        new HtmlWebpackPlugin({
            template: "index.html"
        }),
        new WasmPackPlugin({
            crateDirectory: path.resolve(__dirname, "."),
            // Check https://rustwasm.github.io/wasm-pack/book/commands/build.html for
            // the available set of arguments.
            //
            // Default arguments are `--typescript --target browser --mode normal`.
            extraArgs: "--no-typescript",
            
            // Optional array of absolute paths to directories, changes to which
            // will trigger the build.
            // watchDirectories: [
            //   path.resolve(__dirname, "another-crate/src")
            // ],
            
            // The same as the `--out-dir` option for `wasm-pack`
            // outDir: "pkg",
            
            // The same as the `--out-name` option for `wasm-pack`
            // outName: "index",
            
            // If defined, `forceWatch` will force activate/deactivate watch mode for
            // `.rs` files.
            //
            // The default (not set) aligns watch mode for `.rs` files to Webpack's
            // watch mode.
            // forceWatch: true,
            
            // If defined, `forceMode` will force the compilation mode for `wasm-pack`
            //
            // Possible values are `development` and `production`.
            //
            // the mode `development` makes `wasm-pack` build in `debug` mode.
            // the mode `production` makes `wasm-pack` build in `release` mode.
            forceMode: "production",
        }),
        new CopyWebpackPlugin([
            { from: "static" }
        ]),
        // Have this example work in Edge which doesn't ship `TextEncoder` or
        // `TextDecoder` at this time.
        new webpack.ProvidePlugin({
          TextDecoder: ["text-encoding", "TextDecoder"],
          TextEncoder: ["text-encoding", "TextEncoder"]
        })
    ],
    mode: prod ? "production" : "development"
};