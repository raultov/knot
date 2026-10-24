package com.example.knot.extcalls.app;

import com.example.knot.extcalls.codes.CodeTable;
import org.modelmapper.ModelMapper;

public class ExampleResource {
    private final ModelMapper modelMapper = new ModelMapper();
    private final CodeTable codes = new CodeTable();

    public Object readBean(Object event) {
        return modelMapper.map(event, Object.class);
    }

    public String readId(long id) {
        return String.valueOf(id);
    }

    public String translate(String code) {
        return codes.lookup(code);
    }
}
