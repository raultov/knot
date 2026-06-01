// Fixture for JS circular require alias resolution
// alias_cycle_a.js requires alias_cycle_b.js, which requires alias_cycle_a.js back

var CycleB = require('./alias_cycle_b');

function callerInA() {
    var obj = new CycleB();
}

module.exports = CycleA_target;

function CycleA_target() {}
