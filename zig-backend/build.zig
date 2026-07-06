const std = @import("std");

pub fn build(b: *std.Build) !void {
    const module = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
    });

    const exe = b.addExecutable(.{
        .name = "archetype-mesh-dashboard",
        .root_module = module,
        .linkage = .static,
    });

    b.installArtifact(exe);
}
