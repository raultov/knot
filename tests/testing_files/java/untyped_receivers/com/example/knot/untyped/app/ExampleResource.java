package com.example.knot.untyped.app;

import com.example.knot.untyped.other.LineRenderer;
import org.thirdparty.text.TextRenderer;

/** Calls on `var` receivers whose type the parser cannot always see (issue #42). */
public class ExampleResource {
    /** Third-party mapper: never a caller of the unrelated EventConverter. */
    public Object read(Object event) {
        var mapper = MapperFactory.create();
        return mapper.convertPayload(event, Object.class);
    }

    /** `new` makes the type obvious: a third-party TextRenderer, not LineRenderer. */
    public String stamp() {
        var renderer = new TextRenderer();
        return renderer.renderLine("x");
    }

    public String render(LineRenderer lines) {
        return lines.renderLine("y");
    }

    /** Untyped receiver whose only candidate lives in this package: kept. */
    public int score() {
        var card = ScoreCards.pick();
        return card.computeScore(1);
    }
}
