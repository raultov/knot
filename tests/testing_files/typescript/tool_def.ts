// Fixture for the entry-point recall contract: a tool definition exported
// as an object constant (wire kind `constant`, no callable kind), whose
// name `screenshot` competes with a Markdown section titled the same way
// (docs.md). A natural-language query naming the tool must rank this
// definition above the same-titled prose.
export const screenshot = {
	name: "screenshot",
	capture: "full",
	description: "Capture the visible page area or a selection as an image",
};

export const screenshotElement = {
	name: "elementScreenshot",
	capture: "element",
	description: "Capture a single element chosen by uid as an image",
};
