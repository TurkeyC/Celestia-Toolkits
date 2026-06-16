const Self = @This();
const std = @import("std");
const Config = @import("../config/Config.zig");

allocator: std.mem.Allocator,
config: *Config,
items: std.ArrayList([]const u8),
index: isize,
path: []u8,

pub fn init(allocator: std.mem.Allocator, config: *Config) Self {
    var self = Self{
        .allocator = allocator,
        .config = config,
        .items = .{},
        .index = -1,
        .path = "",
    };

    if (config.general.history <= 0) return self;

    const home = std.process.getEnvVarOwned(allocator, "HOME") catch return self;
    defer allocator.free(home);

    if (!config.legacy_path) {
        const xdg_state_home = std.process.getEnvVarOwned(allocator, "XDG_STATE_HOME") catch null;
        if (xdg_state_home) |x| {
            self.path = std.fmt.allocPrint(allocator, "{s}/fancy-cat/history", .{x}) catch return self;
            allocator.free(x);
        } else self.path = std.fmt.allocPrint(allocator, "{s}/.local/state/fancy-cat/history", .{home}) catch return self;
    } else self.path = std.fmt.allocPrint(allocator, "{s}/.fancy-cat_history", .{home}) catch return self;

    const content = std.fs.cwd().readFileAlloc(allocator, self.path, 1024 * 1024) catch null;
    if (content == null) return self;
    defer allocator.free(content.?);

    var line = std.mem.tokenizeScalar(u8, content.?, '\n');
    while (line.next()) |cmd| {
        const cmd_copy = allocator.dupe(u8, cmd) catch continue;
        self.items.append(allocator, cmd_copy) catch {
            allocator.free(cmd_copy);
            continue;
        };
    }

    return self;
}

pub fn deinit(self: *Self) void {
    if (self.config.general.history <= 0) return;

    defer {
        for (self.items.items) |entry| self.allocator.free(entry);
        self.items.deinit(self.allocator);
        if (self.path.len > 0) self.allocator.free(self.path);
    }

    if (std.fs.path.dirname(self.path)) |dir| std.fs.cwd().makePath(dir) catch {};
    const file = std.fs.createFileAbsolute(self.path, .{}) catch return;
    defer file.close();

    for (self.items.items) |cmd| {
        file.writeAll(cmd) catch continue;
        file.writeAll("\n") catch continue;
    }
}

pub fn addToHistory(self: *Self, cmd: []const u8) void {
    if (self.config.general.history <= 0) return;

    for (self.items.items, 0..) |existing_cmd, i| {
        if (std.mem.eql(u8, existing_cmd, cmd)) {
            self.allocator.free(self.items.orderedRemove(i));
            break;
        }
    }

    const cmd_copy = self.allocator.dupe(u8, cmd) catch return;
    self.items.append(self.allocator, cmd_copy) catch {
        self.allocator.free(cmd_copy);
        return;
    };

    const max: usize = self.config.general.history;
    while (self.items.items.len > max) {
        const removed = self.items.orderedRemove(0);
        self.allocator.free(removed);
    }

    self.index = -1;
}
