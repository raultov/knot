package com.example.knot.http;

@RestController
@RequestMapping("/api")
public class OwnerController {
    @GetMapping({ "/owners", "/owners/" })
    public List<Owner> listOwners() {
        return null;
    }

    @GetMapping(path = "/owners/{ownerId}")
    public Owner showOwner(@PathVariable long ownerId) {
        return null;
    }

    @RequestMapping(value = "/owners/legacy", method = { RequestMethod.GET, RequestMethod.HEAD })
    public void legacyOwners() {
    }
}
