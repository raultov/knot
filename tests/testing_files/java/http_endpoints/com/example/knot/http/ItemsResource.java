package com.example.knot.http;

@Path(Paths.ITEMS)
public class ItemsResource {
    private static final String BY_ID = "/{id: [0-9]+}";

    @GET
    public Response readAll() {
        return null;
    }

    @GET
    @Path(BY_ID)
    public Response read(@PathParam("id") long id) {
        return null;
    }

    @POST
    @PUT
    public Response write(Item item) {
        return null;
    }

    @Path("sub")
    public ItemsResource sub() {
        return this;
    }
}
