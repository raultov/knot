// Fixture for JS cross-file alias resolution (Source)

var MyJsAlias = require('./alias_target_js');

function callerJs() {
    // The call to `new MyJsAlias()` should resolve to `MyJsTarget`
    var obj = new MyJsAlias();
}
