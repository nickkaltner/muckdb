const test = require('node:test');
const assert = require('node:assert/strict');
const { TIMELINE_DEP_ROUTE_PAD, timelineDependencyRoute } = require('../../src/assets/timeline-route.js');

function assertLocal(points, parentBox, childBox, width) {
  assert.ok(points, 'cross-lane dependency has a route');
  const left = Math.max(0, Math.min(parentBox.left, childBox.left) - TIMELINE_DEP_ROUTE_PAD);
  const right = Math.min(width, Math.max(parentBox.right, childBox.right) + TIMELINE_DEP_ROUTE_PAD);
  for (const [x] of points) {
    assert.ok(x >= left, `route x=${x} must stay at or right of ${left}`);
    assert.ok(x <= right, `route x=${x} must stay at or left of ${right}`);
  }
  assert.deepEqual(points[0][0], parentBox.right, 'route leaves the parent back');
  assert.deepEqual(points.at(-1)[0], childBox.left, 'route enters the child front');
}

test('overlapping cross-lane routes stay within the two bars plus a 16px envelope', () => {
  const width = 1000;
  const cases = [
    [{ left: 120, right: 410 }, { left: 220, right: 300 }, 28, 92, 70, 38],
    [{ left: 740, right: 930 }, { left: 680, right: 820 }, 112, 28, 64, 42],
    [{ left: 5, right: 42 }, { left: 15, right: 210 }, 28, 176, 150, 46],
  ];
  for (const [parentBox, childBox, fromY, toY, childLaneTop, childLaneHeight] of cases) {
    const points = timelineDependencyRoute({
      width, parentBox, childBox, fromY, toY, childLaneTop, childLaneHeight,
      overlaps: true, cramped: false, sameLane: false, elbowX: -500, railY: null,
    });
    assertLocal(points, parentBox, childBox, width);
  }
});

test('ordinary and rail routes clamp an over-eager elbow to the same local envelope', () => {
  const width = 1000;
  const parentBox = { left: 420, right: 490 };
  const childBox = { left: 510, right: 670 };
  for (const railY of [null, 14]) {
    const points = timelineDependencyRoute({
      width, parentBox, childBox, fromY: 30, toY: 96, childLaneTop: 72, childLaneHeight: 38,
      overlaps: false, cramped: false, sameLane: false, elbowX: -300, railY,
    });
    assertLocal(points, parentBox, childBox, width);
  }
});

test('a cramped same-lane handoff remains intentionally hidden', () => {
  assert.equal(timelineDependencyRoute({
    width: 600, parentBox: { left: 100, right: 200 }, childBox: { left: 202, right: 280 },
    fromY: 20, toY: 20, childLaneTop: 0, childLaneHeight: 38,
    overlaps: false, cramped: true, sameLane: true, elbowX: 210, railY: null,
  }), null);
});
