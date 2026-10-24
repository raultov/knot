// JavaScript never enforces arity: a call with fewer arguments than declared
// still reaches the function (issue #38).
function legacyPad(text, width) {
  return text + (width || 0);
}

function padDefault(text) {
  return legacyPad(text);
}

module.exports = { padDefault };
