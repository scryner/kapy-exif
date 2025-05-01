#include <iostream>
#include <stdio.h>
#include <math.h>

#include <exiv2/exiv2.hpp>
#include <exiv2/basicio.hpp>

#include "exif.h"

#define EXIF_KEY_GPS_VERSION "Exif.GPSInfo.GPSVersionID"
#define EXIF_KEY_GPS_FORMAT "Exif.GPSInfo.GPSMapDatum"
#define EXIF_KEY_GPS_ALT_REF "Exif.GPSInfo.GPSAltitudeRef"
#define EXIF_KEY_GPS_ALT "Exif.GPSInfo.GPSAltitude"
#define EXIF_KEY_GPS_LAT_REF "Exif.GPSInfo.GPSLatitudeRef"
#define EXIF_KEY_GPS_LAT "Exif.GPSInfo.GPSLatitude"
#define EXIF_KEY_GPS_LON_REF "Exif.GPSInfo.GPSLongitudeRef"
#define EXIF_KEY_GPS_LON "Exif.GPSInfo.GPSLongitude"

// private struct for exif_metadata_t
struct _exif_metadata_private_t
{
    std::unique_ptr<Exiv2::ExifData> data;
    Exiv2::ByteOrder order;
};

// internal functions
char *s_to_cstr(std::string &str);

// make exif_metadata_t
exif_metadata_t *exif_metadata_new()
{
    exif_metadata_t *self = new exif_metadata_t();
    self->priv = new _exif_metadata_private_t();
    self->priv->data = std::make_unique<Exiv2::ExifData>();
    return self;
}

// decode exif metadata from blob (blob starts after "Exif\0\0")
int exif_metadata_from_blob(exif_metadata_t *self, const unsigned char *blob, size_t blob_len)
{
    try
    {
        // create metadata
        Exiv2::ExifData data;

        // decode from blob
        Exiv2::ByteOrder order = Exiv2::ExifParser::decode(data, blob, blob_len);
        *self->priv->data = std::move(data);
        self->priv->order = order;
    }
    catch (Exiv2::Error &e)
    {
        std::cerr << "Failed to read exif from blob: " << e << std::endl;
        return -1;
    }

    return 0;
}

// encode exif metadata to blob (blob hasn't "Exif\0\0" at beginning)
size_t exif_metadata_to_blob(exif_metadata_t *self, unsigned char **out_blob)
{
    size_t out_blob_len = 0;

    try
    {
        Exiv2::Blob blob;
        Exiv2::ExifParser::encode(blob, self->priv->order, *self->priv->data);

        // copy to out_blob
        out_blob_len = blob.size();
        *out_blob = new unsigned char[out_blob_len];
        std::memcpy(*out_blob, blob.data(), out_blob_len);
    }
    catch (Exiv2::Error &e)
    {
        std::cerr << "Failed to write exif to blob: " << e << std::endl;
        return -1;
    }

    return out_blob_len;
}

// get value for given tag
char *exif_get_tag_string(exif_metadata_t *self, const char *tag)
{
    if (self == nullptr || self->priv == nullptr)
    {
        return nullptr;
    }

    try
    {
        // make key to access the metadata
        Exiv2::ExifKey key(tag);

        // read metadata
        Exiv2::ExifData::const_iterator it = self->priv->data->findKey(key);
        if (it != self->priv->data->end())
        {
            std::string val = it->toString();
            return s_to_cstr(val);
        }
        else
        {
            std::cerr << "Failed to find tag: " << tag << std::endl;
            return nullptr;
        }
    }
    catch (...)
    {
        return nullptr;
    }

    return nullptr;
}

// destroy exif metadata
void exif_metadata_destroy(exif_metadata_t **self)
{
    if (self == nullptr || *self == nullptr || (*self)->priv == nullptr)
    {
        return;
    }

    delete (*self)->priv;
    delete *self;
    *self = nullptr;
}

// internal function to convert string to c-string
char *s_to_cstr(std::string &str)
{
    char *ret = new char[str.length() + 1];
    std::strcpy(ret, str.c_str());

    return ret;
}

// add GPS information to exif metadata
int exif_metadata_add_gps_info(exif_metadata_t *self, double lat, double lon, double alt)
{
    try
    {
        Exiv2::ExifData &exif_data = *(self->priv->data);

        // set GPS info version
        Exiv2::ExifKey key(EXIF_KEY_GPS_VERSION);
        Exiv2::ExifData::iterator it = exif_data.findKey(key);
        if (it == exif_data.end())
        {
            exif_data[EXIF_KEY_GPS_VERSION] = "2 0 0 0";
        }

        // set GPS info format
        exif_data[EXIF_KEY_GPS_FORMAT] = "WGS-84";

        // set altitude
        if (alt < 0.0)
        {
            exif_data[EXIF_KEY_GPS_ALT_REF] = "1";
        }
        else
        {
            exif_data[EXIF_KEY_GPS_ALT_REF] = "0";
        }

        Exiv2::Rational frac = Exiv2::floatToRationalCast(static_cast<float>(fabs(alt)));
        exif_data[EXIF_KEY_GPS_ALT] = frac;

        // set latitude
        if (lat < 0.0)
        {
            exif_data[EXIF_KEY_GPS_LAT_REF] = "S";
        }
        else
        {
            exif_data[EXIF_KEY_GPS_LAT_REF] = "N";
        }

        double whole;
        double remainder = modf(fabs(lat), &whole);
        int deg = (int)floor(whole);

        const int denom = 1000000;
        remainder = modf(fabs(remainder * 60), &whole);
        int min = (int)floor(whole);
        int sec = (int)floor(remainder * 60 * denom);

        char buf[100];
        snprintf(buf, 100, "%d/1 %d/1 %d/%d", deg, min, sec, denom);
        exif_data[EXIF_KEY_GPS_LAT] = buf;

        // set longitude
        if (lon < 0.0)
        {
            exif_data[EXIF_KEY_GPS_LON_REF] = "W";
        }
        else
        {
            exif_data[EXIF_KEY_GPS_LON_REF] = "E";
        }

        remainder = modf(fabs(lon), &whole);
        deg = (int)floor(whole);

        remainder = modf(fabs(remainder * 60), &whole);
        min = (int)floor(whole);
        sec = (int)floor(remainder * 60 * denom);

        snprintf(buf, 100, "%d/1 %d/1 %d/%d", deg, min, sec, denom);
        exif_data[EXIF_KEY_GPS_LON] = buf;
    }
    catch (Exiv2::Error &e)
    {
        std::cerr << "Failed to add gps info in exif: " << e << std::endl;
        return -1;
    }

    return 0;
}
