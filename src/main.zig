const std = @import("std");
const Context = @import("Context.zig").Context;

// Types for build.zig.zon
// For now metadata is only used in main.zig, but can move it to types.zig if needed eleswhere
// This wont be necessary once https://github.com/ziglang/zig/pull/22907 is merged

const PackageName = enum { celestia_pdf_reader };

const DependencyType = struct {
    url: []const u8,
    hash: []const u8,
};

const DependenciesType = struct {
    vaxis: DependencyType,
    fzwatch: DependencyType,
    fastb64z: DependencyType,
};

const MetadataType = struct {
    name: PackageName,
    fingerprint: u64,
    version: []const u8,
    minimum_zig_version: []const u8,
    dependencies: DependenciesType,
    paths: []const []const u8,
};

const metadata: MetadataType = @import("metadata");

pub fn main() !void {
    const args = try std.process.argsAlloc(std.heap.page_allocator);
    defer std.process.argsFree(std.heap.page_allocator, args);

    var stdout_buffer: [1024]u8 = undefined;
    var stdout_writer = std.fs.File.stdout().writer(&stdout_buffer);
    const stdout = &stdout_writer.interface;

    var stderr_buffer: [1024]u8 = undefined;
    var stderr_writer = std.fs.File.stderr().writer(&stderr_buffer);
    const stderr = &stderr_writer.interface;

    if (args.len == 2 and (std.mem.eql(u8, args[1], "--version") or std.mem.eql(u8, args[1], "-v"))) {
        try stdout.print("celestia-pdf-reader {s}\n", .{metadata.version});
        try stdout.flush();
        return;
    }

    if (args.len == 2 and (std.mem.eql(u8, args[1], "--help") or std.mem.eql(u8, args[1], "-h"))) {
        try printHelp(stdout);
        try stdout.flush();
        return;
    }

    if (args.len < 2 or args.len > 3) {
        try printUsage(stderr);
        try stderr.flush();
        return;
    }

    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer {
        const deinit_status = gpa.deinit();
        if (deinit_status == .leak) {
            std.log.err("memory leak", .{});
        }
    }
    const allocator = gpa.allocator();

    var app = try Context.init(allocator, args);
    defer app.deinit();

    try app.run();
}

fn printUsage(writer: anytype) !void {
    try writer.writeAll(
        \\Usage: celestia-pdf-reader <file> [page]
        \\Try 'celestia-pdf-reader --help' for more information.
        \\
    );
}

fn printHelp(writer: anytype) !void {
    try writer.writeAll(
        \\celestia-pdf-reader - PDF viewer for terminals using the Kitty image protocol
        \\
        \\Usage: celestia-pdf-reader <file> [page]
        \\
        \\Arguments:
        \\  <file>      Path to a PDF file
        \\  [page]      Page number to open (1-based, default: 1)
        \\
        \\Options:
        \\  -h, --help      Show this help message
        \\  -v, --version   Show version
        \\
        \\Navigation:
        \\  Right/Left    Next/previous page (also n/p)
        \\  j/k           Scroll down/up
        \\  Down/Up       Scroll down/up (also arrow keys)
        \\  h/l           Scroll left/right
        \\  Shift+j/k     Scroll down/up (multiplied)
        \\  Shift+h/l     Scroll left/right (multiplied)
        \\  w             Toggle width/height fit mode
        \\  f             Toggle full screen (hide status bar)
        \\
        \\Zoom:
        \\  =/-           Zoom in/out
        \\  Shift+=/-     Zoom in/out (multiplied)
        \\
        \\Other:
        \\  z             Toggle color replacement
        \\  :             Enter command mode
        \\  q             Quit (also Ctrl+c, Ctrl+q, :q)
        \\
        \\Mouse:
        \\  Scroll        Previous/next page
        \\  Shift+Scroll  Scroll viewport up/down
        \\  Ctrl+Scroll   Zoom in/out
        \\  Left drag     Pan document
        \\
        \\Command mode (press ':'):
        \\  :q            Quit
        \\  :<number>     Go to page number
        \\  :<number>%    Set zoom level
        \\  :x+<number>   Scroll right
        \\  :x-<number>   Scroll left
        \\  :y+<number>   Scroll down
        \\  :y-<number>   Scroll up
        \\
        \\Configuration:
        \\  $XDG_CONFIG_HOME/celestia-pdf-reader/config.json
        \\  $HOME/.config/celestia-pdf-reader/config.json
        \\
        \\Examples:
        \\  celestia-pdf-reader document.pdf
        \\  celestia-pdf-reader document.pdf 42
        \\
    );
}
