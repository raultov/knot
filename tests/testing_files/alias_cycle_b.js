// Fixture for JS circular require alias resolution
// alias_cycle_b.js requires alias_cycle_a.js back — creating a cycle

var CycleA = require('./alias_cycle_a');

function callerInB() {
    var obj = new CycleA();
}

module.exports = CycleB_target;

function CycleB_target() {}
