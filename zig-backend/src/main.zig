const std = @import("std");

pub fn main() !void {
    const stdout_file = std.Io.File.stdout();
    const stderr_file = std.Io.File.stderr();
    const io = std.Io.Threaded.init(std.heap.GeneralPurposeAllocator(.{}){}.allocator(), .{});
    defer std.Io.Threaded.deinit(io);
    var stdout = stdout_file.writer(io, &.{});
    var stderr = stderr_file.writer(io, &.{});

    try stderr.print("Zig foundation: starting\n", .{});
    try stdout.print("Archetype Mesh Benchmark Zig backend\n", .{});
}
