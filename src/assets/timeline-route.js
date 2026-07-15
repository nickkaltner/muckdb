// Pure geometry for timeline dependency connectors.  Keeping this independent
// of the DOM makes the routing invariants inexpensive to test.
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.timelineDependencyRoute = api.timelineDependencyRoute;
  root.TIMELINE_DEP_ROUTE_PAD = api.TIMELINE_DEP_ROUTE_PAD;
})(typeof globalThis === "undefined" ? this : globalThis, function () {
  // A connector needs 10px to visibly leave a bar, plus the timeline's 6px
  // lane padding.  It must never use the distant label gutter as a detour.
  const TIMELINE_DEP_ROUTE_PAD = 16;
  const EXIT_RUN = 10;
  const ENTRY_RUN = 7;

  const clamp = (n, lo, hi) => Math.max(lo, Math.min(hi, n));

  // Returns orthogonal points from the parent's back (right edge) to the
  // child's front (left edge), or null for a deliberately hidden same-lane
  // handoff. Every point stays within the union of the two bar boxes ± PAD.
  function timelineDependencyRoute({
    width, parentBox, childBox, fromY, toY, childLaneTop, childLaneHeight,
    cramped, sameLane, overlaps, elbowX, railY,
  }) {
    if (cramped && sameLane) return null;

    const minX = clamp(Math.min(parentBox.left, childBox.left) - TIMELINE_DEP_ROUTE_PAD, 0, width);
    const maxX = clamp(Math.max(parentBox.right, childBox.right) + TIMELINE_DEP_ROUTE_PAD, 0, width);
    const fromX = clamp(parentBox.right, minX, maxX);
    const toX = clamp(childBox.left, minX, maxX);
    const exitX = clamp(fromX + EXIT_RUN, minX, maxX);
    const entryX = clamp(toX - ENTRY_RUN, minX, maxX);

    if (overlaps || cramped) {
      // Enter the destination through its adjacent lane gutter.  The mirrored
      // paths form the downward/upward backwards-S without a long trip into
      // the label gutter on the left of the tile.
      const gutterY = toY >= fromY
        ? Math.max(2, childLaneTop - 2)
        : childLaneTop + childLaneHeight - 2;
      return [[fromX, fromY], [exitX, fromY], [exitX, gutterY], [entryX, gutterY], [entryX, toY], [toX, toY]];
    }

    const elbow = clamp(elbowX, minX, maxX);
    return railY == null
      ? [[fromX, fromY], [elbow, fromY], [elbow, toY], [toX, toY]]
      : [[fromX, fromY], [elbow, fromY], [elbow, railY], [toX, railY], [toX, toY]];
  }

  return { TIMELINE_DEP_ROUTE_PAD, timelineDependencyRoute };
});
