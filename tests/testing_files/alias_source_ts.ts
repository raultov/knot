// Fixture for TS cross-file alias resolution (Source)

import { MyTsTarget as MyTsAlias } from './alias_target_ts';

function callerTs() {
    // The call to `new MyTsAlias()` should resolve to `MyTsTarget`
    let obj = new MyTsAlias();
}
